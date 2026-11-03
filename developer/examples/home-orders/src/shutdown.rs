//! Process shutdown: signal handling, the shutdown channel every component can
//! watch, and cleanup hooks for clients and consumers.
//!
//! Wiring, from `main`:
//!
//! 1. [`channel`] creates a trigger + watch pair;
//! 2. a spawned task awaits [`wait_for_signal`] and triggers it;
//! 3. the HTTP server watches it and drains in-flight requests (see
//!    [`crate::server::serve`]);
//! 4. [`CleanupRegistry`] hooks (database pool close in ticket #5, cache and
//!    AMQP cleanup later) run after the server has stopped.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::sync::watch;

/// How long in-flight HTTP requests may take to finish after a shutdown
/// signal, before the remaining connections are aborted.
pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Per-task budget for [`CleanupRegistry`] hooks.
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Signals a process shutdown to every watcher.
#[derive(Clone)]
pub struct ShutdownTrigger(watch::Sender<bool>);

/// Watches for process shutdown; clone it for every component that needs it.
#[derive(Clone)]
pub struct ShutdownWatch(watch::Receiver<bool>);

/// Creates the shutdown channel.
pub fn channel() -> (ShutdownTrigger, ShutdownWatch) {
    let (sender, receiver) = watch::channel(false);
    (ShutdownTrigger(sender), ShutdownWatch(receiver))
}

impl ShutdownTrigger {
    /// Fires the shutdown signal; later calls are no-ops.
    pub fn trigger(&self) {
        self.0.send_replace(true);
    }
}

impl ShutdownWatch {
    /// Completes when shutdown has been signalled.
    pub async fn wait(&mut self) {
        if *self.0.borrow() {
            return;
        }
        // Sender dropped without triggering means nobody can trigger: return.
        let _ = self.0.wait_for(|signalled| *signalled).await;
    }

    /// Whether shutdown has already been signalled.
    pub fn is_signalled(&self) -> bool {
        *self.0.borrow()
    }
}

/// Completes when SIGINT (Ctrl-C) or SIGTERM arrives.
pub async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                tracing::error!(error = %error, "failed to install the SIGTERM handler; waiting for SIGINT only");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

type CleanupTask = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Startup-registered cleanup hooks, run after the HTTP server has stopped.
#[derive(Default)]
pub struct CleanupRegistry {
    tasks: Vec<(&'static str, CleanupTask)>,
}

impl CleanupRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a cleanup task; `name` appears in the shutdown logs.
    pub fn register<F>(&mut self, name: &'static str, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tasks.push((name, Box::pin(task)));
    }

    /// Runs every task in registration order; a task that outlives
    /// `task_timeout` is abandoned and logged.
    pub async fn run_all(self, task_timeout: Duration) {
        for (name, task) in self.tasks {
            match tokio::time::timeout(task_timeout, task).await {
                Ok(()) => tracing::info!(cleanup = name, "cleanup task finished"),
                Err(_) => tracing::warn!(
                    cleanup = name,
                    timeout_ms = task_timeout.as_millis(),
                    "cleanup task timed out; abandoning it"
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use super::*;

    #[tokio::test]
    async fn watch_wakes_every_watcher_on_trigger() {
        let (trigger, mut first) = channel();
        let mut second = first.clone();

        let waiter = tokio::spawn(async move {
            first.wait().await;
            second.wait().await;
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        trigger.trigger();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("watchers wake up")
            .unwrap();
    }

    #[tokio::test]
    async fn watch_returns_immediately_when_already_signalled() {
        let (trigger, mut watch) = channel();
        trigger.trigger();

        assert!(watch.is_signalled());
        tokio::time::timeout(Duration::from_secs(1), watch.wait())
            .await
            .expect("wait returns immediately");
    }

    #[tokio::test]
    async fn cleanup_runs_tasks_and_bounds_slow_ones() {
        let finished = Arc::new(AtomicBool::new(false));
        let mut registry = CleanupRegistry::new();
        {
            let finished = Arc::clone(&finished);
            registry.register("quick", async move {
                finished.store(true, Ordering::SeqCst);
            });
        }
        registry.register("stuck", std::future::pending::<()>());

        let start = Instant::now();
        registry.run_all(Duration::from_millis(50)).await;

        assert!(finished.load(Ordering::SeqCst), "quick task must run");
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "run_all must not wait forever, took {:?}",
            start.elapsed()
        );
    }
}
