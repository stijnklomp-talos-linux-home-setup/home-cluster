//! GraphQL federation feature tests (ADR-0011): schema SDL, entity resolution,
//! JWT extraction, authorization guards and data-loader batching. Runs only
//! with `--features graphql` (see the `[[test]]` entry in `Cargo.toml`).

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
use home_users::config::{AppConfig, GraphQLConfig};
use home_users::graphql::GraphQLState;
use home_users::graphql::auth::AuthContext;
use home_users::graphql::schema::{self, UserLoader};
use home_users::models::user::User as UserRow;
use home_users::problem::PROBLEM_JSON_CONTENT_TYPE;
use home_users::repositories::users::{RepositoryError, RepositoryFuture, UsersRepository};
use home_users::server::{AppState, build_router};
use home_users::services::users::UsersService;
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use tower::ServiceExt;

const TEST_KEY: &[u8] = include_bytes!("fixtures/graphql-test-key.pem");
const OTHER_TEST_KEY: &[u8] = include_bytes!("fixtures/graphql-test-key-2.pem");
const KID: &str = "test-key";
const ISSUER: &str = "https://issuer.test";
const AUDIENCE: &str = "home-users";
const CLIENT_ID: &str = "home-users";

/// The three reference users served by the fake repository.
struct FakeUsersRepository;

impl UsersRepository for FakeUsersRepository {
    fn find_page(&self, _page: u32, _per_page: u32) -> RepositoryFuture<'_, Vec<UserRow>> {
        Box::pin(async { Ok(sample_users()) })
    }

    fn find_by_id(&self, id: i64) -> RepositoryFuture<'_, Option<UserRow>> {
        let user = sample_users().into_iter().find(|user| user.id == id);
        Box::pin(async move { Ok(user) })
    }

    fn find_by_ids(&self, ids: &[i64]) -> RepositoryFuture<'_, Vec<UserRow>> {
        let users = sample_users()
            .into_iter()
            .filter(|user| ids.contains(&user.id))
            .collect();
        Box::pin(async move { Ok(users) })
    }
}

fn sample_users() -> Vec<UserRow> {
    let created_at = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        NaiveTime::from_hms_micro_opt(12, 0, 0, 123_456).unwrap(),
    );
    vec![
        UserRow {
            id: 1,
            name: "Ada Lovelace".to_owned(),
            email: "ada@example.test".to_owned(),
            created_at,
        },
        UserRow {
            id: 2,
            name: "Grace Hopper".to_owned(),
            email: "grace@example.test".to_owned(),
            created_at,
        },
        UserRow {
            id: 3,
            name: "Alan Turing".to_owned(),
            email: "alan@example.test".to_owned(),
            created_at,
        },
    ]
}

fn users_service() -> Arc<UsersService> {
    Arc::new(UsersService::new(Arc::new(FakeUsersRepository)))
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
            "email": "ada@example.test",
            "email_verified": true,
            "scope": "users:read",
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
            claims["email_verified"] = json!(true);
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

    /// Token for user 1 (Ada) with full read scopes.
    fn privileged_token(&self) -> String {
        self.token(
            ISSUER,
            Some(AUDIENCE),
            None,
            "1",
            Some("ada@example.test"),
            "users:read users:write",
        )
    }

    /// Cognito-shaped token with no `email` claim and an email `username`.
    fn username_token(&self) -> String {
        let claims = json!({
            "sub": "1",
            "iss": ISSUER,
            "aud": AUDIENCE,
            "exp": jsonwebtoken::get_current_timestamp() + 300,
            "username": "ada@example.test",
            "scope": "orders:read",
        });
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(KID.to_owned());
        encode(&header, &claims, &self.encoding_key).expect("sign test token")
    }

    /// Token for user 1 (Ada) without `users:read`.
    fn self_only_token(&self) -> String {
        self.token(
            ISSUER,
            Some(AUDIENCE),
            None,
            "1",
            Some("ada@example.test"),
            "",
        )
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
    let state = AppState {
        graphql: Some(GraphQLState::new(&config, users_service())),
        users: Some(users_service()),
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
async fn sdl_declares_federation_v2_scopes_and_the_user_key() {
    let idp = Idp::start().await;
    let token = idp.privileged_token();

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
        sdl.contains(r#"type User @key(fields: "id")"#),
        "SDL must declare the User entity key: {sdl}"
    );
    assert!(
        sdl.contains("email: String!"),
        "SDL must expose email: {sdl}"
    );
    assert!(
        !sdl.contains(r#"email: String! @requiresScopes"#),
        "email must not carry @requiresScopes: self-email would be unreachable through Hive: {sdl}"
    );
    assert!(
        sdl.contains(r#"users(page: Int! = 1, perPage: Int! = 100): [User!]! @requiresScopes(scopes: [["users:read"]])"#),
        "users must require users:read: {sdl}"
    );
    assert!(
        sdl.contains("createdAt: String!"),
        "SDL must expose createdAt: {sdl}"
    );
    assert!(
        sdl.contains("initials: String!"),
        "SDL must expose the computed initials field: {sdl}"
    );
}

#[tokio::test]
async fn computed_initials_field_is_served_and_federated() {
    let idp = Idp::start().await;
    let token = idp.privileged_token();

    let response = post_graphql(
        app(idp.config()),
        Some(&token),
        json!({ "query": "{ users { id name initials } }" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body.get("errors").is_none(), "{body}");
    assert_eq!(body["data"]["users"][0]["initials"], "AL");
    assert_eq!(body["data"]["users"][1]["initials"], "GH");
    assert_eq!(body["data"]["users"][2]["initials"], "AT");

    // The field is also resolvable through the federation entity path, which
    // is the path Hive uses when another subgraph extends User.
    let response = post_graphql(
        app(idp.config()),
        Some(&token),
        json!({
            "query": "query($representations: [_Any!]!) { \
                _entities(representations: $representations) { ... on User { id initials } } \
            }",
            "variables": {
                "representations": [{ "__typename": "User", "id": "2" }]
            }
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body.get("errors").is_none(), "{body}");
    assert_eq!(body["data"]["_entities"][0]["initials"], "GH");
}

#[tokio::test]
async fn users_query_returns_users_with_scope() {
    let idp = Idp::start().await;
    let token = idp.privileged_token();

    let response = post_graphql(
        app(idp.config()),
        Some(&token),
        json!({ "query": "{ users(page: 1, perPage: 10) { id name email } }" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body.get("errors").is_none(), "{body}");
    assert_eq!(body["data"]["users"][0]["id"], "1");
    assert_eq!(body["data"]["users"][0]["name"], "Ada Lovelace");
    assert_eq!(body["data"]["users"][0]["email"], "ada@example.test");
    assert_eq!(body["data"]["users"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn users_query_without_scope_is_denied_by_the_service() {
    let idp = Idp::start().await;
    let token = idp.self_only_token();

    let response = post_graphql(
        app(idp.config()),
        Some(&token),
        json!({ "query": "{ users { id } }" }),
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
            .contains("users:read"),
        "{body}"
    );
}

#[tokio::test]
async fn user_query_is_visible_to_self_or_with_scope() {
    let idp = Idp::start().await;

    // Self without users:read: own record (including own email) is visible,
    // but a different user resolves to null instead of leaking existence.
    let response = post_graphql(
        app(idp.config()),
        Some(&idp.self_only_token()),
        json!({
            "query": "{ me: user(id: \"1\") { id name email } other: user(id: \"2\") { id name } }"
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body.get("errors").is_none(), "{body}");
    assert_eq!(body["data"]["me"]["id"], "1");
    assert_eq!(body["data"]["me"]["email"], "ada@example.test");
    assert!(body["data"]["other"].is_null(), "{body}");

    // users:read sees any user.
    let response = post_graphql(
        app(idp.config()),
        Some(&idp.privileged_token()),
        json!({ "query": "{ user(id: \"2\") { id name email } }" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["user"]["id"], "2");
    assert_eq!(body["data"]["user"]["email"], "grace@example.test");
}

#[tokio::test]
async fn entity_email_requires_scope_or_self() {
    let idp = Idp::start().await;

    let query = json!({
        "query": "query($representations: [_Any!]!) { \
            _entities(representations: $representations) { ... on User { id email } } \
        }",
        "variables": {
            "representations": [{ "__typename": "User", "id": "2" }]
        }
    });
    let response = post_graphql(app(idp.config()), Some(&idp.self_only_token()), query).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let errors = body["errors"].as_array().expect("errors");
    assert!(
        errors[0]["message"]
            .as_str()
            .unwrap()
            .contains("users:read"),
        "{body}"
    );

    // The caller's own entity email is allowed without the scope.
    let query = json!({
        "query": "query($representations: [_Any!]!) { \
            _entities(representations: $representations) { ... on User { id email } } \
        }",
        "variables": {
            "representations": [{ "__typename": "User", "id": "1" }]
        }
    });
    let response = post_graphql(app(idp.config()), Some(&idp.self_only_token()), query).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body.get("errors").is_none(), "{body}");
    assert_eq!(body["data"]["_entities"][0]["email"], "ada@example.test");
}

#[tokio::test]
async fn entities_resolve_the_sample_user() {
    let idp = Idp::start().await;
    let token = idp.privileged_token();

    let response = post_graphql(
        app(idp.config()),
        Some(&token),
        json!({
            "query": "query($representations: [_Any!]!) { \
                _entities(representations: $representations) { ... on User { id name email } } \
            }",
            "variables": {
                "representations": [{ "__typename": "User", "id": "2" }]
            }
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["_entities"][0]["id"], "2");
    assert_eq!(body["data"]["_entities"][0]["name"], "Grace Hopper");
    assert_eq!(body["data"]["_entities"][0]["email"], "grace@example.test");
}

#[tokio::test]
async fn valid_token_populates_the_graphql_context() {
    let idp = Idp::start().await;
    let token = idp.privileged_token();

    let response = post_graphql(
        app(idp.config()),
        Some(&token),
        json!({ "query": "{ viewer { subject email emailVerified userId scopes } }" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["viewer"]["subject"], "1");
    assert_eq!(body["data"]["viewer"]["email"], "ada@example.test");
    assert_eq!(body["data"]["viewer"]["emailVerified"], true);
    assert_eq!(body["data"]["viewer"]["userId"], 1);
    assert_eq!(
        body["data"]["viewer"]["scopes"],
        json!(["users:read", "users:write"])
    );
}

#[tokio::test]
async fn cognito_username_is_an_email_fallback() {
    let idp = Idp::start().await;

    let response = post_graphql(
        app(idp.config()),
        Some(&idp.username_token()),
        json!({ "query": "{ viewer { email userId } }" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["viewer"]["email"], "ada@example.test");
}

#[tokio::test]
async fn missing_token_is_problem_json_401() {
    let idp = Idp::start().await;

    let response = post_graphql(
        app(idp.config()),
        None,
        json!({ "query": "{ viewer { subject } }" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        PROBLEM_JSON_CONTENT_TYPE
    );
    let body = body_json(response).await;
    assert_eq!(body["title"], "Unauthorized");
    assert_eq!(body["detail"], "missing bearer token");
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
            json!({ "query": "{ viewer { subject } }" }),
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

    // A valid token for user 1 plus X-User/X-Email for user 2 must still be
    // user 1: the service only reads the Authorization header.
    let response = post_graphql_with_headers(
        app(idp.config()),
        Some(&idp.privileged_token()),
        &[("x-user", "2"), ("x-email", "grace@example.test")],
        json!({ "query": "{ viewer { subject email userId } }" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["viewer"]["userId"], 1);
    assert_eq!(body["data"]["viewer"]["email"], "ada@example.test");

    // Spoofed identity headers without a token do not authenticate.
    let response = post_graphql_with_headers(
        app(idp.config()),
        None,
        &[("x-user", "1"), ("x-email", "ada@example.test")],
        json!({ "query": "{ viewer { subject } }" }),
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
        "",
    );
    let response = post_graphql(
        app(config.clone()),
        Some(&token),
        json!({ "query": "{ viewer { userId } }" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["viewer"]["userId"], 1);

    for bad in [
        idp.token(ISSUER, None, Some("another-client"), "1", None, ""),
        idp.token(ISSUER, None, None, "1", None, ""),
    ] {
        let response = post_graphql(
            app(config.clone()),
            Some(&bad),
            json!({ "query": "{ viewer { userId } }" }),
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
    let token = idp.privileged_token();
    let mut config = idp.config();
    config.jwks_url = "http://127.0.0.1:1/jwks".to_owned();

    let response = post_graphql(
        app(config),
        Some(&token),
        json!({ "query": "{ viewer { subject } }" }),
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
    let users = users_service();
    let loader = UserLoader::new(users.clone());
    let stats = loader.stats();
    let schema = schema::schema(users);

    // One `_entities` call with two representations resolves both entities
    // concurrently; the loader must see a single batched `load`.
    let response = schema
        .execute(
            Request::new(
                r#"{
                    _entities(representations: [
                        {__typename: "User", id: "1"},
                        {__typename: "User", id: "3"}
                    ]) { ... on User { id } }
                }"#,
            )
            .data(DataLoader::new(loader, tokio::spawn)),
        )
        .await;

    assert!(response.errors.is_empty(), "{:?}", response.errors);
    assert_eq!(
        response.data,
        value!({ "_entities": [{ "id": "1" }, { "id": "3" }] })
    );
    assert_eq!(stats.loads(), 1, "the two lookups must share one load call");
    assert_eq!(stats.keys(), 2);
}

#[tokio::test]
async fn self_email_requires_a_verified_email_claim() {
    let idp = Idp::start().await;
    let unverified = idp.token_with(json!({
        "sub": "cognito-uuid",
        "iss": ISSUER,
        "aud": AUDIENCE,
        "exp": jsonwebtoken::get_current_timestamp() + 300,
        "email": "ada@example.test",
        "scope": "",
    }));

    // An unverified email must not make user 1 "self": the single-user query
    // hides the record and the entity email stays denied.
    let response = post_graphql(
        app(idp.config()),
        Some(&unverified),
        json!({ "query": "{ user(id: \"1\") { id email } }" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body.get("errors").is_none(), "{body}");
    assert!(body["data"]["user"].is_null(), "{body}");

    let query = json!({
        "query": "query($representations: [_Any!]!) { \
            _entities(representations: $representations) { ... on User { id email } } \
        }",
        "variables": { "representations": [{ "__typename": "User", "id": "1" }] }
    });
    let response = post_graphql(app(idp.config()), Some(&unverified), query).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(
        body["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("users:read"),
        "{body}"
    );

    // The same token with `email_verified: true` is the caller.
    let verified = idp.token_with(json!({
        "sub": "cognito-uuid",
        "iss": ISSUER,
        "aud": AUDIENCE,
        "exp": jsonwebtoken::get_current_timestamp() + 300,
        "email": "ada@example.test",
        "email_verified": true,
        "scope": "",
    }));
    let response = post_graphql(
        app(idp.config()),
        Some(&verified),
        json!({ "query": "{ user(id: \"1\") { id email } }" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body.get("errors").is_none(), "{body}");
    assert_eq!(body["data"]["user"]["email"], "ada@example.test");
}

#[tokio::test]
async fn missing_token_is_problem_json_401_with_a_bearer_challenge() {
    let idp = Idp::start().await;

    let response = post_graphql(
        app(idp.config()),
        None,
        json!({ "query": "{ viewer { subject } }" }),
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
                claims(json!({ "scope": "users:read" })),
            ),
        ),
        (
            "unknown kid",
            idp.sign(
                &idp.encoding_key,
                "rotated-key",
                Algorithm::RS256,
                claims(json!({ "scope": "users:read" })),
            ),
        ),
        (
            "non-RS256 algorithm",
            idp.sign(
                &hmac_key,
                KID,
                Algorithm::HS256,
                claims(json!({ "scope": "users:read" })),
            ),
        ),
        ("missing exp", {
            let mut claims = claims(json!({ "scope": "users:read" }));
            claims.as_object_mut().unwrap().remove("exp");
            idp.token_with(claims)
        }),
    ];

    for (case, token) in tokens {
        let response = post_graphql(
            app(idp.config()),
            Some(&token),
            json!({ "query": "{ viewer { subject } }" }),
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
            json!({ "query": "{ viewer { subject } }" }),
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
            post_graphql(
                app,
                Some(&token),
                json!({ "query": "{ viewer { subject } }" }),
            )
            .await
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
        Some(&idp.privileged_token()),
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
        Some(&idp.privileged_token()),
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
            format!("Bearer {}", idp.privileged_token()),
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
    let auth = AuthContext {
        subject: Some("1".to_owned()),
        email: Some("ada@example.test".to_owned()),
        email_verified: true,
        user_id: Some(1),
        scopes: vec!["users:read".to_owned()],
    };
    let response = schema::schema(users_service())
        .execute(Request::new(r#"{ user(id: "1") { id } }"#).data(auth))
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
    assert_eq!(config.graphql.audience.as_deref(), Some("home-users"));
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
        "GRAPHQL_JWT_CLIENT_ID" => Some("home-users".to_owned()),
        _ => None,
    })
    .expect("config");

    assert_eq!(config.graphql.audience, None);
    assert_eq!(config.graphql.client_id.as_deref(), Some("home-users"));
}

/// Keeps the fake repository's error type referenced so a future failing fake
/// stays a one-line change.
#[allow(dead_code)]
fn repository_error_is_available() -> RepositoryError {
    RepositoryError::Database(sqlx::Error::RowNotFound)
}
