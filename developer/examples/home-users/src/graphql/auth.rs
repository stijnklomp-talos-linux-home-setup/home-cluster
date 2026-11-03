//! JWT authentication for the GraphQL subgraph (ADR-0011).
//!
//! Every GraphQL request must carry a bearer token. [`JwtVerifier`] validates
//! the signature against the configured JWKS, plus the issuer, expiry and the
//! configured audience/`client_id` claims; [`AuthContext`] is the resulting
//! caller, inserted into the GraphQL context by the handler. Missing or
//! invalid tokens are rejected at the HTTP layer with problem+json 401 and a
//! `WWW-Authenticate: Bearer` challenge; a JWKS fetch failure is a 503.
//!
//! Cognito *access* tokens carry `client_id` and a space-delimited `scope`
//! claim, but no `aud`; set `GRAPHQL_JWT_AUDIENCE` to an empty string and
//! configure `GRAPHQL_JWT_CLIENT_ID` instead. ID tokens (which do carry
//! `aud`) can keep the audience check. The `email` claim (and the `username`
//! fallback) identifies the caller for self-access only when the token also
//! carries `email_verified: true`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};

use crate::config::GraphQLConfig;
use crate::problem::Problem;
use crate::server::AppState;

/// How long a JWKS request may take before authentication is unavailable.
const JWKS_TIMEOUT: Duration = Duration::from_secs(5);

/// Minimum interval between JWKS fetches. An unknown `kid` inside this window
/// is rejected instead of triggering another identity-provider request, so a
/// forged `kid` cannot turn every request into a JWKS fetch.
const JWKS_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// The algorithm this service accepts. The identity providers in scope
/// (Keycloak, Cognito) issue RS256 access tokens.
const ACCEPTED_ALGORITHM: Algorithm = Algorithm::RS256;

const BEARER_CHALLENGE: &str = "Bearer";

/// Validated caller, available to every resolver through the GraphQL context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthContext {
    /// The token's `sub` claim.
    pub subject: Option<String>,
    /// The token's `email` claim (or an email-shaped `username` fallback).
    pub email: Option<String>,
    /// Whether the token's email claim is verified. Only a verified address
    /// may identify the caller for self-access.
    pub email_verified: bool,
    /// The caller's local user id: the `user_id` claim, the `custom:user_id`
    /// Cognito attribute, or a numeric `sub`.
    pub user_id: Option<i64>,
    /// Scopes from the `scope` and `scp` claims, sorted and deduplicated.
    pub scopes: Vec<String>,
}

impl AuthContext {
    /// Whether the token grants `scope`.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|granted| granted == scope)
    }

    /// Whether this caller is the user with `id`/`email`, by local id or a
    /// **verified** (case-insensitive) email address.
    pub fn is_self(&self, id: i64, email: &str) -> bool {
        self.user_id == Some(id)
            || (self.email_verified
                && self
                    .email
                    .as_deref()
                    .is_some_and(|caller| caller.eq_ignore_ascii_case(email)))
    }
}

/// Claims this service reads; signature, issuer, audience/`client_id` and
/// expiry are validated by [`jsonwebtoken`] before this struct is produced.
#[derive(Debug, Clone, Deserialize)]
struct Claims {
    #[serde(default)]
    sub: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_verified: Option<serde_json::Value>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    user_id: Option<serde_json::Value>,
    #[serde(default, rename = "custom:user_id")]
    custom_user_id: Option<serde_json::Value>,
    #[serde(default)]
    scope: Option<ScopeClaim>,
    #[serde(default)]
    scp: Option<ScopeClaim>,
}

/// Scope claims come as a space-delimited string (OIDC `scope`, Keycloak) or
/// as a string array (Azure `scp`); accept both on either claim name. Array
/// members are split on whitespace too, because issuers are inconsistent.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ScopeClaim {
    SpaceDelimited(String),
    Array(Vec<String>),
}

impl ScopeClaim {
    fn into_scopes(self) -> Vec<String> {
        match self {
            Self::SpaceDelimited(value) => split_scopes(&value),
            Self::Array(values) => values
                .iter()
                .flat_map(|value| split_scopes(value))
                .collect(),
        }
    }
}

fn split_scopes(value: &str) -> Vec<String> {
    value.split_whitespace().map(ToOwned::to_owned).collect()
}

impl Claims {
    fn into_context(self) -> AuthContext {
        let mut scopes = self.scp.map(ScopeClaim::into_scopes).unwrap_or_default();
        if let Some(scope) = self.scope {
            scopes.extend(scope.into_scopes());
        }
        scopes.sort();
        scopes.dedup();

        let user_id = claim_i64(&self.user_id)
            .or_else(|| claim_i64(&self.custom_user_id))
            .or_else(|| self.sub.as_deref().and_then(|sub| sub.parse().ok()));

        // Cognito access tokens do not carry `email` by default; when the
        // pool's username is the address, `username` is the usable fallback.
        let email = self
            .email
            .or_else(|| self.username.filter(|username| username.contains('@')));

        AuthContext {
            subject: self.sub,
            email,
            email_verified: claim_bool(&self.email_verified),
            user_id,
            scopes,
        }
    }
}

/// Parses a numeric claim that Cognito may carry as a JSON number or string.
fn claim_i64(value: &Option<serde_json::Value>) -> Option<i64> {
    match value {
        Some(serde_json::Value::Number(number)) => number.as_i64(),
        Some(serde_json::Value::String(value)) => value.parse().ok(),
        _ => None,
    }
}

/// Parses a boolean claim that Cognito may carry as a JSON bool or a string.
fn claim_bool(value: &Option<serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Bool(value)) => *value,
        Some(serde_json::Value::String(value)) => value.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// Why a bearer token was rejected. The detail stays in the logs; responses
/// only say whether the token was missing or invalid.
#[derive(Debug, Error)]
pub enum AuthError {
    /// No `Authorization: Bearer <token>` header.
    #[error("missing bearer token")]
    MissingToken,
    /// The token failed signature or claim validation.
    #[error("invalid bearer token")]
    InvalidToken,
    /// The JWKS endpoint could not be reached or returned an unusable set.
    #[error("JWKS unavailable: {0}")]
    JwksUnavailable(String),
}

/// Verifies bearer tokens against a lazily fetched, cached JWKS.
///
/// The key set is fetched on the first request (never at startup, so an
/// unavailable identity provider cannot block boot) and refreshed when a
/// token references an unknown `kid`, which covers key rotation. Refreshes
/// are single-flight and rate-limited to one per [`JWKS_REFRESH_INTERVAL`].
#[derive(Clone)]
pub struct JwtVerifier {
    inner: Arc<VerifierInner>,
}

struct VerifierInner {
    issuer: String,
    audience: Option<String>,
    client_id: Option<String>,
    jwks_url: String,
    http: reqwest::Client,
    keys: RwLock<JwkSet>,
    refresh: Mutex<RefreshState>,
}

/// Timestamp of the last JWKS fetch (successful or not), guarded by
/// `VerifierInner::refresh` so concurrent requests share one fetch.
#[derive(Debug, Default)]
struct RefreshState {
    last_attempt: Option<Instant>,
}

impl JwtVerifier {
    /// Builds a verifier from `GRAPHQL_*` configuration.
    pub fn new(config: &GraphQLConfig) -> Self {
        // The timeout is part of the contract (a hung IdP must not hang
        // authentication); a client that cannot be built is a programming
        // error, so surface it instead of silently dropping the timeout.
        let http = reqwest::Client::builder()
            .timeout(JWKS_TIMEOUT)
            .build()
            .expect("build the JWKS HTTP client");

        Self {
            inner: Arc::new(VerifierInner {
                issuer: config.issuer.clone(),
                audience: config.audience.clone(),
                client_id: config.client_id.clone(),
                jwks_url: config.jwks_url.clone(),
                http,
                keys: RwLock::new(JwkSet::default()),
                refresh: Mutex::new(RefreshState::default()),
            }),
        }
    }

    /// Verifies `token`, returning the caller context on success.
    pub async fn verify(&self, token: &str) -> Result<AuthContext, AuthError> {
        let header = decode_header(token).map_err(|error| {
            tracing::debug!(error = %error, "GraphQL bearer token header is invalid");
            AuthError::InvalidToken
        })?;
        if header.alg != ACCEPTED_ALGORITHM {
            tracing::debug!(
                algorithm = ?header.alg,
                expected = ?ACCEPTED_ALGORITHM,
                "GraphQL bearer token algorithm rejected"
            );
            return Err(AuthError::InvalidToken);
        }
        let Some(kid) = header.kid.as_deref() else {
            tracing::debug!("GraphQL bearer token has no kid");
            return Err(AuthError::InvalidToken);
        };
        let jwk = self.key(kid).await?;
        let key = DecodingKey::from_jwk(&jwk).map_err(|error| {
            tracing::debug!(error = %error, "GraphQL JWKS key is unusable");
            AuthError::InvalidToken
        })?;

        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[self.inner.issuer.as_str()]);
        if let Some(audience) = self.inner.audience.as_deref() {
            validation.set_audience(&[audience]);
            validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        } else {
            // Cognito access tokens have no `aud`; jsonwebtoken rejects a
            // present `aud` when audience validation is enabled but unset.
            validation.validate_aud = false;
            validation.set_required_spec_claims(&["exp", "iss"]);
        }

        let claims = match decode::<Claims>(token, &key, &validation) {
            Ok(data) => data.claims,
            Err(error) => {
                // The cause stays here: `AuthError::InvalidToken` never leaks
                // token or validation details into the response.
                tracing::debug!(error = %error, "GraphQL bearer token failed validation");
                return Err(AuthError::InvalidToken);
            }
        };

        if let Some(expected) = self.inner.client_id.as_deref()
            && claims.client_id.as_deref() != Some(expected)
        {
            tracing::debug!(
                client_id = ?claims.client_id,
                "GraphQL bearer token client_id rejected"
            );
            return Err(AuthError::InvalidToken);
        }

        Ok(claims.into_context())
    }

    /// Returns the JWK for `kid`, refreshing the cached set on a miss.
    async fn key(&self, kid: &str) -> Result<Jwk, AuthError> {
        if let Some(jwk) = self.cached(kid).await {
            return Ok(jwk);
        }

        // Single-flight: waiters re-check the cache, then the refresh window,
        // under the same mutex, so exactly one request fetches per interval.
        let mut refresh = self.inner.refresh.lock().await;
        if let Some(jwk) = self.cached(kid).await {
            return Ok(jwk);
        }
        if refresh
            .last_attempt
            .is_some_and(|last| last.elapsed() < JWKS_REFRESH_INTERVAL)
        {
            tracing::debug!(
                kid,
                "GraphQL JWKS refresh suppressed by the minimum interval"
            );
            return Err(AuthError::InvalidToken);
        }
        refresh.last_attempt = Some(Instant::now());

        let keys = self.fetch_keys().await?;
        *self.inner.keys.write().await = keys;
        self.cached(kid).await.ok_or(AuthError::InvalidToken)
    }

    async fn cached(&self, kid: &str) -> Option<Jwk> {
        self.inner.keys.read().await.find(kid).cloned()
    }

    async fn fetch_keys(&self) -> Result<JwkSet, AuthError> {
        let response = self
            .inner
            .http
            .get(&self.inner.jwks_url)
            .send()
            .await
            .map_err(|error| AuthError::JwksUnavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(AuthError::JwksUnavailable(format!(
                "{} returned {}",
                self.inner.jwks_url,
                response.status()
            )));
        }
        response
            .json()
            .await
            .map_err(|error| AuthError::JwksUnavailable(error.to_string()))
    }
}

/// Extracts and validates the bearer token for a GraphQL request.
impl FromRequestParts<AppState> for AuthContext {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let graphql = state.graphql().map_err(|_| {
            Problem::new(StatusCode::SERVICE_UNAVAILABLE)
                .with_detail("GraphQL is not configured")
                .into_response()
        })?;
        let token = bearer_token(parts).ok_or_else(|| unauthorized("missing bearer token"))?;

        graphql
            .verifier
            .verify(token)
            .await
            .map_err(|error| match error {
                AuthError::JwksUnavailable(detail) => {
                    tracing::error!(error = %detail, "GraphQL JWKS fetch failed");
                    Problem::new(StatusCode::SERVICE_UNAVAILABLE)
                        .with_detail("authentication temporarily unavailable")
                        .into_response()
                }
                other => {
                    tracing::debug!(error = %other, "GraphQL bearer token rejected");
                    unauthorized(other.to_string())
                }
            })
    }
}

/// RFC 7235 challenge for a 401: clients know which scheme to retry with.
fn unauthorized(detail: impl Into<String>) -> Response {
    let mut response = Problem::new(StatusCode::UNAUTHORIZED)
        .with_detail(detail)
        .into_response();
    response.headers_mut().insert(
        WWW_AUTHENTICATE,
        BEARER_CHALLENGE
            .parse()
            .expect("static bearer challenge is a valid header value"),
    );
    response
}

fn bearer_token(parts: &Parts) -> Option<&str> {
    let value = parts.headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scope_and_scp_accept_space_delimited_and_array_forms() {
        let claims: Claims = serde_json::from_value(json!({
            "sub": "alice",
            "scope": ["users:read", "users:write orders:read"],
            "scp": "orders:admin users:read",
        }))
        .expect("mixed scope forms");

        assert_eq!(
            claims.into_context().scopes,
            vec!["orders:admin", "orders:read", "users:read", "users:write"]
        );
    }

    #[test]
    fn scope_claim_is_optional() {
        let claims: Claims = serde_json::from_value(json!({ "sub": "alice" })).unwrap();
        let context = claims.into_context();

        assert_eq!(context.subject.as_deref(), Some("alice"));
        assert!(context.scopes.is_empty());
        assert!(!context.email_verified);
    }

    #[test]
    fn email_self_access_requires_a_verified_address() {
        let unverified = serde_json::from_value::<Claims>(json!({
            "sub": "cognito-uuid",
            "email": "ada@example.test",
        }))
        .unwrap()
        .into_context();
        assert!(
            !unverified.is_self(2, "ada@example.test"),
            "an unverified email must not identify the caller"
        );

        let verified = serde_json::from_value::<Claims>(json!({
            "sub": "ada@example.test",
            "email": "ada@example.test",
            "email_verified": "true",
        }))
        .unwrap()
        .into_context();
        assert!(verified.is_self(2, "ADA@example.test"));
        assert!(!verified.is_self(2, "grace@example.test"));
    }
}
