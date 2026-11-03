//! GraphQL federation feature tests (ADR-0011, ticket #16): schema SDL, the
//! `Order` entity, the `User` reference, JWT extraction, ownership guards and
//! data-loader batching. Runs only with `--features graphql` (see the
//! `[[test]]` entry in `Cargo.toml`).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_graphql::Request;
use async_graphql::dataloader::DataLoader;
use async_graphql::value;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{Request as HttpRequest, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use home_orders::config::{AppConfig, GraphQLConfig};
use home_orders::graphql::GraphQLState;
use home_orders::graphql::auth::AuthContext;
use home_orders::graphql::schema::{self, OrderLoader};
use home_orders::models::order::Order as OrderRow;
use home_orders::problem::PROBLEM_JSON_CONTENT_TYPE;
use home_orders::repositories::orders::{OrdersRepository, RepositoryError, RepositoryFuture};
use home_orders::server::{AppState, build_router};
use home_orders::services::orders::OrdersService;
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use tower::ServiceExt;

const TEST_KEY: &[u8] = include_bytes!("fixtures/graphql-test-key.pem");
const OTHER_TEST_KEY: &[u8] = include_bytes!("fixtures/graphql-test-key-2.pem");
const KID: &str = "test-key";
const ISSUER: &str = "https://issuer.test";
const AUDIENCE: &str = "home-orders";
const CLIENT_ID: &str = "home-orders";

/// The three reference orders served by the fake repository, across three
/// different users (the cross-service path needs more than one key).
struct FakeOrdersRepository {
    batch_calls: AtomicUsize,
}

impl FakeOrdersRepository {
    fn new() -> Self {
        Self {
            batch_calls: AtomicUsize::new(0),
        }
    }
}

impl OrdersRepository for FakeOrdersRepository {
    fn find_page(&self, _page: u32, _per_page: u32) -> RepositoryFuture<'_, Vec<OrderRow>> {
        Box::pin(async { Ok(sample_orders()) })
    }

    fn find_by_id(&self, id: i64) -> RepositoryFuture<'_, Option<OrderRow>> {
        let order = sample_orders().into_iter().find(|order| order.id == id);
        Box::pin(async move { Ok(order) })
    }

    fn find_by_ids(&self, ids: &[i64]) -> RepositoryFuture<'_, Vec<OrderRow>> {
        self.batch_calls.fetch_add(1, Ordering::Relaxed);
        let orders = sample_orders()
            .into_iter()
            .filter(|order| ids.contains(&order.id))
            .collect();
        Box::pin(async move { Ok(orders) })
    }
}

fn sample_orders() -> Vec<OrderRow> {
    let created_at = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        NaiveTime::from_hms_micro_opt(12, 0, 0, 123_456).unwrap(),
    );
    vec![
        OrderRow {
            id: 1,
            user_id: 1,
            total: 12_500,
            status: "paid".to_owned(),
            created_at,
        },
        OrderRow {
            id: 2,
            user_id: 2,
            total: 89_900,
            status: "pending".to_owned(),
            created_at,
        },
        OrderRow {
            id: 3,
            user_id: 3,
            total: 250,
            status: "shipped".to_owned(),
            created_at,
        },
    ]
}

fn orders_service() -> Arc<OrdersService> {
    Arc::new(OrdersService::new(Arc::new(FakeOrdersRepository::new())))
}

fn auth(user_id: Option<i64>, email: Option<&str>, scopes: &[&str]) -> AuthContext {
    AuthContext {
        subject: user_id.map(|id| id.to_string()),
        email: email.map(ToOwned::to_owned),
        user_id,
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
    }
}

/// A local identity provider: an RSA key, a JWKS endpoint serving its public
/// half, and a token minting helper.
struct Idp {
    encoding_key: EncodingKey,
    jwks_url: String,
    jwks_hits: Arc<AtomicUsize>,
}

impl Idp {
    async fn start() -> Self {
        let encoding_key = EncodingKey::from_rsa_pem(TEST_KEY).expect("test RSA key");
        let (jwks_url, jwks_hits) = serve_jwks(JwkSet {
            keys: vec![test_jwk(TEST_KEY, KID)],
        })
        .await;

        Self {
            encoding_key,
            jwks_url,
            jwks_hits,
        }
    }

    fn claims(&self) -> Value {
        json!({
            "sub": "1",
            "iss": ISSUER,
            "aud": AUDIENCE,
            "exp": jsonwebtoken::get_current_timestamp() + 300,
            "scope": "orders:read",
        })
    }

    /// Signs a token. `audience` and `client_id` are optional so the Cognito
    /// access-token shape (no `aud`) can be exercised.
    #[allow(clippy::too_many_arguments)]
    fn token(
        &self,
        issuer: &str,
        audience: Option<&str>,
        client_id: Option<&str>,
        sub: &str,
        email: Option<&str>,
        scope: &str,
    ) -> String {
        let mut claims = json!({
            "sub": sub,
            "iss": issuer,
            "exp": jsonwebtoken::get_current_timestamp() + 300,
            "scope": scope,
        });
        if let Some(audience) = audience {
            claims["aud"] = json!(audience);
        }
        if let Some(client_id) = client_id {
            claims["client_id"] = json!(client_id);
        }
        if let Some(email) = email {
            claims["email"] = json!(email);
        }

        self.sign(&self.encoding_key, KID, Algorithm::RS256, claims)
    }

    fn token_with(&self, claims: Value) -> String {
        self.sign(&self.encoding_key, KID, Algorithm::RS256, claims)
    }

    fn sign(&self, key: &EncodingKey, kid: &str, alg: Algorithm, claims: Value) -> String {
        let mut header = Header::new(alg);
        header.kid = Some(kid.to_owned());
        encode(&header, &claims, key).expect("sign test token")
    }

    /// Number of requests the JWKS endpoint has served.
    fn jwks_hits(&self) -> usize {
        self.jwks_hits.load(Ordering::SeqCst)
    }

    /// Token for user 1 (Ada) with `orders:read`.
    fn reader_token(&self) -> String {
        self.token(
            ISSUER,
            Some(AUDIENCE),
            None,
            "1",
            Some("ada@example.test"),
            "orders:read",
        )
    }

    /// Token for user 1 (Ada) with `orders:admin`.
    fn admin_token(&self) -> String {
        self.token(
            ISSUER,
            Some(AUDIENCE),
            None,
            "1",
            Some("ada@example.test"),
            "orders:admin",
        )
    }

    /// Token for user 1 (Ada) with no orders scope.
    fn unscoped_token(&self) -> String {
        self.token(ISSUER, Some(AUDIENCE), None, "1", None, "")
    }

    fn config(&self) -> GraphQLConfig {
        GraphQLConfig {
            enabled: true,
            jwks_url: self.jwks_url.clone(),
            issuer: ISSUER.to_owned(),
            audience: Some(AUDIENCE.to_owned()),
            client_id: None,
        }
    }
}

fn test_jwk(pem: &[u8], kid: &str) -> Jwk {
    let key = EncodingKey::from_rsa_pem(pem).expect("test RSA key");
    let mut jwk = Jwk::from_encoding_key(&key, Algorithm::RS256).expect("public JWK");
    jwk.common.key_id = Some(kid.to_owned());
    jwk
}

/// Serves `jwks_set` at `GET /jwks` on an ephemeral localhost port and counts
/// the requests, so tests can assert the verifier did not re-fetch.
async fn serve_jwks(jwks_set: JwkSet) -> (String, Arc<AtomicUsize>) {
    #[derive(Clone)]
    struct JwksState {
        jwks: JwkSet,
        hits: Arc<AtomicUsize>,
    }

    async fn jwks_handler(State(state): State<JwksState>) -> Json<JwkSet> {
        state.hits.fetch_add(1, Ordering::SeqCst);
        Json(state.jwks)
    }

    let hits = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind JWKS listener");
    let address = listener.local_addr().expect("JWKS address");
    let app = Router::new()
        .route("/jwks", get(jwks_handler))
        .with_state(JwksState {
            jwks: jwks_set,
            hits: Arc::clone(&hits),
        });
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve JWKS");
    });

    (format!("http://{address}/jwks"), hits)
}

fn app(config: GraphQLConfig) -> Router {
    app_with_service(config, orders_service())
}

fn app_with_service(config: GraphQLConfig, orders: Arc<OrdersService>) -> Router {
    let state = AppState {
        graphql: Some(GraphQLState::new(&config, orders.clone())),
        orders: Some(orders),
        ..AppState::default()
    };
    build_router(state)
}

async fn post_graphql(app: Router, token: Option<&str>, body: Value) -> Response {
    post_graphql_with_headers(app, token, &[], body).await
}

async fn post_graphql_with_headers(
    app: Router,
    token: Option<&str>,
    extra_headers: &[(&str, &str)],
    body: Value,
) -> Response {
    let mut request = HttpRequest::builder()
        .method("POST")
        .uri("/graphql")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    for (name, value) in extra_headers {
        request = request.header(*name, *value);
    }

    app.oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

async fn post_graphql_raw(
    app: Router,
    token: Option<&str>,
    content_type: &str,
    body: Body,
) -> Response {
    let mut request = HttpRequest::builder()
        .method("POST")
        .uri("/graphql")
        .header(header::CONTENT_TYPE, content_type);
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }

    app.oneshot(request.body(body).unwrap()).await.unwrap()
}

async fn body_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn assert_problem(response: Response, status: StatusCode, detail: &str) -> Value {
    assert_eq!(response.status(), status);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        PROBLEM_JSON_CONTENT_TYPE
    );
    let body = body_json(response).await;
    assert_eq!(body["status"], status.as_u16());
    assert_eq!(body["detail"], detail);
    body
}

#[tokio::test]
async fn sdl_declares_federation_v2_scopes_and_the_order_entity() {
    let idp = Idp::start().await;
    let token = idp.reader_token();

    let response = post_graphql(
        app(idp.config()),
        Some(&token),
        json!({ "query": "{ _service { sdl } }" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let sdl = body["data"]["_service"]["sdl"].as_str().expect("SDL");

    assert!(
        sdl.contains("https://specs.apollo.dev/federation/v2."),
        "SDL must link Federation v2: {sdl}"
    );
    assert!(
        sdl.contains(r#"type Order @key(fields: "id")"#),
        "SDL must declare the Order entity key: {sdl}"
    );
    assert!(sdl.contains("userId: ID!"), "SDL must expose userId: {sdl}");
    assert!(sdl.contains("total: Int!"), "SDL must expose total: {sdl}");
    assert!(
        sdl.contains(r#"type User @key(fields: "id", resolvable: false)"#),
        "the User reference must be an unresolvable entity: {sdl}"
    );
    assert!(
        sdl.contains("user: User!"),
        "Order.user must return the User reference: {sdl}"
    );
    assert!(
        sdl.contains(r#"orders(page: Int! = 1, perPage: Int! = 100): [Order!]! @requiresScopes(scopes: [["orders:read"], ["orders:admin"]])"#),
        "orders must require an orders scope: {sdl}"
    );
    assert!(
        sdl.contains(
            r#"order(id: ID!): Order @requiresScopes(scopes: [["orders:read"], ["orders:admin"]])"#
        ),
        "order must require an orders scope: {sdl}"
    );
}

#[tokio::test]
async fn orders_query_returns_only_the_callers_orders() {
    let idp = Idp::start().await;
    let token = idp.reader_token();

    let response = post_graphql(
        app(idp.config()),
        Some(&token),
        json!({ "query": "{ orders(page: 1, perPage: 10) { id total status userId } }" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body.get("errors").is_none(), "{body}");
    let orders = body["data"]["orders"].as_array().unwrap();
    assert_eq!(orders.len(), 1, "{body}");
    assert_eq!(orders[0]["id"], "1");
    assert_eq!(orders[0]["userId"], "1");
}

#[tokio::test]
async fn orders_admin_sees_every_order() {
    let idp = Idp::start().await;

    let response = post_graphql(
        app(idp.config()),
        Some(&idp.admin_token()),
        json!({ "query": "{ orders { id } }" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body.get("errors").is_none(), "{body}");
    assert_eq!(body["data"]["orders"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn orders_query_without_scope_is_denied_by_the_service() {
    let idp = Idp::start().await;

    let response = post_graphql(
        app(idp.config()),
        Some(&idp.unscoped_token()),
        json!({ "query": "{ orders { id } }" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body["data"].is_null(), "{body}");
    let errors = body["errors"].as_array().expect("errors");
    assert!(
        errors[0]["message"]
            .as_str()
            .unwrap()
            .contains("orders:read"),
        "{body}"
    );
}

#[tokio::test]
async fn order_query_denies_a_different_owner() {
    let idp = Idp::start().await;
    let token = idp.reader_token();

    let response = post_graphql(
        app(idp.config()),
        Some(&token),
        json!({ "query": "{ own: order(id: \"1\") { id userId } other: order(id: \"2\") { id userId } }" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body.get("errors").is_none(), "{body}");
    assert_eq!(body["data"]["own"]["id"], "1");
    assert!(body["data"]["other"].is_null(), "{body}");
}

#[tokio::test]
async fn entities_deny_a_different_owners_order() {
    let idp = Idp::start().await;

    let query = json!({
        "query": "query($representations: [_Any!]!) { \
            _entities(representations: $representations) { ... on Order { id userId } } \
        }",
        "variables": {
            "representations": [{ "__typename": "Order", "id": "2" }]
        }
    });
    let response = post_graphql(app(idp.config()), Some(&idp.reader_token()), query).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body["data"]["_entities"][0].is_null(), "{body}");

    let query = json!({
        "query": "query($representations: [_Any!]!) { \
            _entities(representations: $representations) { ... on Order { id userId } } \
        }",
        "variables": {
            "representations": [{ "__typename": "Order", "id": "1" }]
        }
    });
    let response = post_graphql(app(idp.config()), Some(&idp.reader_token()), query).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["_entities"][0]["id"], "1");
}

#[tokio::test]
async fn order_query_returns_one_order_and_its_user_reference() {
    let idp = Idp::start().await;
    let token = idp.reader_token();

    let response = post_graphql(
        app(idp.config()),
        Some(&token),
        json!({ "query": "{ order(id: \"1\") { id total status createdAt user { id } } }" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body.get("errors").is_none(), "{body}");
    assert_eq!(body["data"]["order"]["id"], "1");
    assert_eq!(body["data"]["order"]["total"], 12_500);
    assert_eq!(body["data"]["order"]["status"], "paid");
    assert_eq!(
        body["data"]["order"]["createdAt"],
        "2026-11-01T12:00:00.123456"
    );
    // The user is a federation reference: only the key comes from orders.
    assert_eq!(body["data"]["order"]["user"]["id"], "1");
    assert_eq!(
        body["data"]["order"]["user"].as_object().unwrap().len(),
        1,
        "only the key may be returned here: {body}"
    );
}

#[tokio::test]
async fn missing_token_is_problem_json_401() {
    let idp = Idp::start().await;

    let response = post_graphql(
        app(idp.config()),
        None,
        json!({ "query": "{ orders { id } }" }),
    )
    .await;

    assert_eq!(
        response.headers()[header::WWW_AUTHENTICATE],
        "Bearer",
        "a 401 must carry a WWW-Authenticate challenge"
    );
    let body = assert_problem(response, StatusCode::UNAUTHORIZED, "missing bearer token").await;
    assert_eq!(body["title"], "Unauthorized");
}

#[tokio::test]
async fn invalid_token_is_problem_json_401() {
    let idp = Idp::start().await;
    let wrong_issuer = idp.token(
        "https://other-issuer.test",
        Some(AUDIENCE),
        None,
        "1",
        None,
        "",
    );
    let wrong_audience = idp.token(ISSUER, Some("another-service"), None, "1", None, "");

    for token in ["not-a-jwt", wrong_issuer.as_str(), wrong_audience.as_str()] {
        let response = post_graphql(
            app(idp.config()),
            Some(token),
            json!({ "query": "{ orders { id } }" }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{token}");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            PROBLEM_JSON_CONTENT_TYPE
        );
        let body = body_json(response).await;
        assert_eq!(body["detail"], "invalid bearer token", "{token}");
    }
}

#[tokio::test]
async fn spoofed_identity_headers_are_ignored() {
    let idp = Idp::start().await;

    // A valid token for user 1 plus X-User/X-Email for user 2 must still see
    // only user 1's orders: the service reads the Authorization header alone.
    let response = post_graphql_with_headers(
        app(idp.config()),
        Some(&idp.reader_token()),
        &[("x-user", "2"), ("x-email", "grace@example.test")],
        json!({ "query": "{ orders { userId } }" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["orders"].as_array().unwrap().len(), 1);
    assert_eq!(body["data"]["orders"][0]["userId"], "1");

    // Spoofed identity headers without a token do not authenticate.
    let response = post_graphql_with_headers(
        app(idp.config()),
        None,
        &[("x-user", "1")],
        json!({ "query": "{ orders { id } }" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn cognito_access_token_without_aud_is_validated_by_client_id() {
    let idp = Idp::start().await;
    let mut config = idp.config();
    config.audience = None;
    config.client_id = Some(CLIENT_ID.to_owned());

    let token = idp.token(
        ISSUER,
        None,
        Some(CLIENT_ID),
        "1",
        Some("ada@example.test"),
        "orders:read",
    );
    let response = post_graphql(
        app(config.clone()),
        Some(&token),
        json!({ "query": "{ orders { id } }" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["orders"].as_array().unwrap().len(), 1);

    for bad in [
        idp.token(
            ISSUER,
            None,
            Some("another-client"),
            "1",
            None,
            "orders:read",
        ),
        idp.token(ISSUER, None, None, "1", None, "orders:read"),
    ] {
        let response = post_graphql(
            app(config.clone()),
            Some(&bad),
            json!({ "query": "{ orders { id } }" }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = body_json(response).await;
        assert_eq!(body["detail"], "invalid bearer token");
    }
}

#[tokio::test]
async fn unavailable_jwks_is_problem_json_503() {
    let idp = Idp::start().await;
    let token = idp.reader_token();
    let mut config = idp.config();
    config.jwks_url = "http://127.0.0.1:1/jwks".to_owned();

    let response = post_graphql(
        app(config),
        Some(&token),
        json!({ "query": "{ orders { id } }" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        PROBLEM_JSON_CONTENT_TYPE
    );
    let body = body_json(response).await;
    assert_eq!(body["detail"], "authentication temporarily unavailable");
}

#[tokio::test]
async fn graphql_route_is_absent_when_disabled() {
    let response = build_router(AppState::default())
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/graphql")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"query":"{ __typename }"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        PROBLEM_JSON_CONTENT_TYPE
    );
}

#[tokio::test]
async fn dataloader_batches_entity_lookups() {
    let orders = orders_service();
    let loader = OrderLoader::new(orders.clone());
    let stats = loader.stats();
    let schema = schema::schema(orders);

    // One `_entities` call with three representations resolves them
    // concurrently; the loader must see a single batched `load`. The admin
    // context is allowed to resolve every owner's order.
    let response = schema
        .execute(
            Request::new(
                r#"{
                    _entities(representations: [
                        {__typename: "Order", id: "1"},
                        {__typename: "Order", id: "2"},
                        {__typename: "Order", id: "3"}
                    ]) { ... on Order { id user { id } } }
                }"#,
            )
            .data(DataLoader::new(loader, tokio::spawn))
            .data(auth(Some(1), None, &["orders:admin"])),
        )
        .await;

    assert!(response.errors.is_empty(), "{:?}", response.errors);
    assert_eq!(
        response.data,
        value!({ "_entities": [
            { "id": "1", "user": { "id": "1" } },
            { "id": "2", "user": { "id": "2" } },
            { "id": "3", "user": { "id": "3" } }
        ] })
    );
    assert_eq!(
        stats.loads(),
        1,
        "the three lookups must share one load call"
    );
    assert_eq!(stats.keys(), 3);
}

#[tokio::test]
async fn orders_query_does_not_batch_fetch_orders() {
    // `orders` is one list query; the repository is called once, not once per
    // row, and the user references are resolved by the users subgraph.
    let repository = Arc::new(FakeOrdersRepository::new());
    let service = Arc::new(OrdersService::new(repository.clone()));
    let idp = Idp::start().await;

    let response = post_graphql(
        app_with_service(idp.config(), service),
        Some(&idp.admin_token()),
        json!({ "query": "{ orders { id user { id } } }" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["orders"].as_array().unwrap().len(), 3);
    assert_eq!(
        repository.batch_calls.load(Ordering::Relaxed),
        0,
        "the list query must not fan out into per-order entity loads"
    );
}

#[tokio::test]
async fn expired_wrong_key_unknown_kid_alg_and_missing_exp_are_401() {
    let idp = Idp::start().await;
    let now = jsonwebtoken::get_current_timestamp();
    let claims = |extra: Value| {
        let mut claims = idp.claims();
        claims
            .as_object_mut()
            .expect("object claims")
            .extend(extra.as_object().expect("object extra").clone());
        claims
    };
    let other_key = EncodingKey::from_rsa_pem(OTHER_TEST_KEY).expect("second test RSA key");
    let hmac_key = EncodingKey::from_secret(b"not-the-signing-key");

    let tokens = [
        (
            "expired",
            idp.token_with(claims(json!({ "exp": now - 300 }))),
        ),
        (
            "wrong signing key",
            idp.sign(
                &other_key,
                KID,
                Algorithm::RS256,
                claims(json!({ "scope": "orders:read" })),
            ),
        ),
        (
            "unknown kid",
            idp.sign(
                &idp.encoding_key,
                "rotated-key",
                Algorithm::RS256,
                claims(json!({ "scope": "orders:read" })),
            ),
        ),
        (
            "non-RS256 algorithm",
            idp.sign(
                &hmac_key,
                KID,
                Algorithm::HS256,
                claims(json!({ "scope": "orders:read" })),
            ),
        ),
        ("missing exp", {
            let mut claims = claims(json!({ "scope": "orders:read" }));
            claims.as_object_mut().unwrap().remove("exp");
            idp.token_with(claims)
        }),
    ];

    for (case, token) in tokens {
        let response = post_graphql(
            app(idp.config()),
            Some(&token),
            json!({ "query": "{ orders { id } }" }),
        )
        .await;

        assert_eq!(
            response.headers()[header::WWW_AUTHENTICATE],
            "Bearer",
            "{case}"
        );
        assert_problem(response, StatusCode::UNAUTHORIZED, "invalid bearer token").await;
    }
}

#[tokio::test]
async fn unknown_kid_does_not_refetch_the_jwks_within_the_refresh_window() {
    let idp = Idp::start().await;
    let token = idp.sign(
        &idp.encoding_key,
        "unknown-kid",
        Algorithm::RS256,
        idp.claims(),
    );
    let app = app(idp.config());

    for _ in 0..2 {
        let response = post_graphql(
            app.clone(),
            Some(&token),
            json!({ "query": "{ orders { id } }" }),
        )
        .await;

        assert_problem(response, StatusCode::UNAUTHORIZED, "invalid bearer token").await;
    }

    assert_eq!(
        idp.jwks_hits(),
        1,
        "the second unknown-kid request must not fetch the JWKS again"
    );
}

#[tokio::test]
async fn concurrent_unknown_kids_share_one_jwks_fetch() {
    let idp = Idp::start().await;
    let token = idp.sign(
        &idp.encoding_key,
        "unknown-kid",
        Algorithm::RS256,
        idp.claims(),
    );
    let app = app(idp.config());

    let mut handles = Vec::new();
    for _ in 0..4 {
        let app = app.clone();
        let token = token.clone();
        handles.push(tokio::spawn(async move {
            post_graphql(app, Some(&token), json!({ "query": "{ orders { id } }" })).await
        }));
    }

    for handle in handles {
        let response = handle.await.expect("request task");
        assert_problem(response, StatusCode::UNAUTHORIZED, "invalid bearer token").await;
    }

    assert_eq!(
        idp.jwks_hits(),
        1,
        "concurrent refreshes must be single-flight"
    );
}

#[tokio::test]
async fn malformed_json_body_is_problem_json_400() {
    let idp = Idp::start().await;

    let response = post_graphql_raw(
        app(idp.config()),
        Some(&idp.reader_token()),
        "application/json",
        Body::from("{\"query\": "),
    )
    .await;

    assert_problem(
        response,
        StatusCode::BAD_REQUEST,
        "malformed GraphQL request",
    )
    .await;
}

#[tokio::test]
async fn multipart_without_a_boundary_is_problem_json_400() {
    let idp = Idp::start().await;

    let response = post_graphql_raw(
        app(idp.config()),
        Some(&idp.reader_token()),
        "multipart/form-data",
        Body::empty(),
    )
    .await;

    assert_problem(
        response,
        StatusCode::BAD_REQUEST,
        "malformed GraphQL request",
    )
    .await;
}

#[tokio::test]
async fn oversized_graphql_body_is_problem_json_413() {
    let idp = Idp::start().await;

    let request = HttpRequest::builder()
        .method("POST")
        .uri("/graphql")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", idp.reader_token()),
        )
        .header(header::CONTENT_LENGTH, 1024 * 1024 + 1)
        .body(Body::empty())
        .unwrap();
    let response = app(idp.config()).oneshot(request).await.unwrap();

    assert_problem(
        response,
        StatusCode::PAYLOAD_TOO_LARGE,
        "request body exceeds the 1048576 byte limit",
    )
    .await;
}

#[tokio::test]
async fn missing_dataloader_is_a_graphql_error_not_a_panic() {
    let response = schema::schema(orders_service())
        .execute(Request::new(r#"{ order(id: "1") { id } }"#).data(auth(
            Some(1),
            None,
            &["orders:read"],
        )))
        .await;

    assert_eq!(response.data, value!(null));
    assert_eq!(response.errors.len(), 1, "{:?}", response.errors);
    assert!(
        response.errors[0].message.contains("does not exist"),
        "{:?}",
        response.errors[0].message
    );
}

#[test]
fn invalid_graphql_urls_fail_fast() {
    for value in ["", "not-a-url", "https://", "ftp://idp.example/certs"] {
        let config = AppConfig::from_lookup(|key| match key {
            "DATABASE_URL" => Some("postgres://user:pass@localhost:5432/db".to_owned()),
            "GRAPHQL_JWKS_URL" | "GRAPHQL_JWT_ISSUER" => Some(value.to_owned()),
            _ => None,
        });
        assert!(config.is_err(), "{value} must be rejected");
    }
}

#[test]
fn graphql_config_defaults_are_local_and_enabled() {
    let config = AppConfig::from_lookup(|key| {
        (key == "DATABASE_URL").then(|| "postgres://user:pass@localhost:5432/db".to_owned())
    })
    .expect("config");

    assert!(config.graphql.enabled);
    assert_eq!(config.graphql.issuer, "http://localhost:8080/realms/home");
    assert_eq!(config.graphql.audience.as_deref(), Some("home-orders"));
    assert_eq!(config.graphql.client_id, None);
    assert!(
        config
            .graphql
            .jwks_url
            .ends_with("/protocol/openid-connect/certs")
    );
}

#[test]
fn empty_audience_selects_the_cognito_access_token_mode() {
    let config = AppConfig::from_lookup(|key| match key {
        "DATABASE_URL" => Some("postgres://user:pass@localhost:5432/db".to_owned()),
        "GRAPHQL_JWT_AUDIENCE" => Some(String::new()),
        "GRAPHQL_JWT_CLIENT_ID" => Some("home-orders".to_owned()),
        _ => None,
    })
    .expect("config");

    assert_eq!(config.graphql.audience, None);
    assert_eq!(config.graphql.client_id.as_deref(), Some("home-orders"));
}

/// Keeps the fake repository's error type referenced so a future failing fake
/// stays a one-line change.
#[allow(dead_code)]
fn repository_error_is_available() -> RepositoryError {
    RepositoryError::Database(sqlx::Error::RowNotFound)
}
