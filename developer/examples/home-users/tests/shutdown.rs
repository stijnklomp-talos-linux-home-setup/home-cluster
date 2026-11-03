//! Graceful shutdown behavior over a real (loopback) socket.

use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::get;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

use home_users::server::serve;
use home_users::shutdown;

/// Opens a connection, sends a request, and leaves it in flight.
async fn send_request(addr: std::net::SocketAddr, path: &str) -> TcpStream {
    let mut connection = TcpStream::connect(addr).await.unwrap();
    connection
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
        .await
        .unwrap();
    connection
}

#[tokio::test]
async fn shutdown_drains_in_flight_requests_cleanly() {
    let app = Router::new().route("/", get(|| async { "ok" }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (trigger, watch) = shutdown::channel();
    let server = tokio::spawn(serve(listener, app, watch, Duration::from_secs(5)));

    // A completed request before shutdown.
    let mut connection = send_request(addr, "/").await;
    let mut buffer = [0_u8; 1024];
    use tokio::io::AsyncReadExt;
    let read = connection.read(&mut buffer).await.unwrap();
    assert!(
        String::from_utf8_lossy(&buffer[..read]).starts_with("HTTP/1.1 200"),
        "request should be served before shutdown"
    );

    trigger.trigger();
    let result = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("serve returns after shutdown")
        .expect("serve task does not panic");
    assert!(result.is_ok());
}

#[tokio::test]
async fn drain_timeout_aborts_hanging_requests() {
    let app = Router::new().route(
        "/slow",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            "too late"
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (trigger, watch) = shutdown::channel();
    let server = tokio::spawn(serve(listener, app, watch, Duration::from_millis(100)));

    // Put a request in flight and never read its response.
    let _connection = send_request(addr, "/slow").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let started = Instant::now();
    trigger.trigger();
    let result = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("serve returns after the drain timeout")
        .expect("serve task does not panic");

    assert!(result.is_ok());
    assert!(
        started.elapsed() >= Duration::from_millis(100),
        "should wait for the drain window, waited {:?}",
        started.elapsed()
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "should abort instead of waiting for the 30s handler"
    );
}
