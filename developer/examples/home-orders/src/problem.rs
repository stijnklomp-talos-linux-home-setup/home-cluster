//! The one error contract: RFC 9457 `application/problem+json` (ADR-0005).
//!
//! Every non-2xx response the app produces goes through [`Problem`], whether it
//! comes from a handler returning [`ApiError`] or from the router fallback.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Media type of every error response.
pub const PROBLEM_JSON_CONTENT_TYPE: &str = "application/problem+json";

/// `type` member used when no more specific problem type applies.
pub const ABOUT_BLANK: &str = "about:blank";

/// RFC 9457 problem details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Problem {
    /// URI identifying the problem type.
    #[serde(rename = "type")]
    pub type_uri: String,
    /// Short human-readable summary of the problem type.
    pub title: String,
    /// HTTP status code.
    pub status: u16,
    /// Human-readable explanation for this occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// URI of the specific occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Validation details (extension member, ADR-0005).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ProblemError>>,
}

/// One entry in the `errors` validation extension member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProblemError {
    /// What is wrong with the value.
    pub detail: String,
    /// JSON Pointer of the offending field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

impl Problem {
    /// Problem for `status` with its canonical title.
    pub fn new(status: StatusCode) -> Self {
        Self {
            type_uri: ABOUT_BLANK.to_owned(),
            title: status.canonical_reason().unwrap_or("Unknown").to_owned(),
            status: status.as_u16(),
            detail: None,
            instance: None,
            errors: None,
        }
    }

    /// Sets the `type` member.
    pub fn with_type(mut self, type_uri: impl Into<String>) -> Self {
        self.type_uri = type_uri.into();
        self
    }

    /// Overrides the canonical title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Sets the occurrence-specific detail.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Sets the occurrence URI.
    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    /// Sets the `errors` validation extension.
    pub fn with_errors(mut self, errors: Vec<ProblemError>) -> Self {
        self.errors = Some(errors);
        self
    }

    fn status_code(&self) -> StatusCode {
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let mut response = Json(self).into_response();
        *response.status_mut() = status;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(PROBLEM_JSON_CONTENT_TYPE),
        );
        response
    }
}

/// Errors a handler returns; every variant maps to a [`Problem`] response.
#[derive(Debug, Error)]
pub enum ApiError {
    /// 404: the resource does not exist.
    #[error("not found")]
    NotFound,
    /// 400: the request is malformed.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// 401: authentication is missing or invalid.
    #[error("unauthorized")]
    Unauthorized,
    /// 403: the caller is not allowed to do this.
    #[error("forbidden")]
    Forbidden,
    /// 409: the request conflicts with current state.
    #[error("conflict: {0}")]
    Conflict(String),
    /// 503: a dependency is unavailable.
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
    /// 500: something failed; the source is logged, never exposed.
    #[error("internal server error")]
    Internal {
        /// Underlying cause, logged at error level.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl ApiError {
    /// HTTP status this error maps to.
    pub fn status(&self) -> StatusCode {
        self.parts().0
    }

    /// Converts into the response body; internal details are never exposed.
    pub fn into_problem(self) -> Problem {
        let (status, detail) = self.parts();
        match detail {
            Some(detail) => Problem::new(status).with_detail(detail.to_owned()),
            None => Problem::new(status),
        }
    }

    /// Status and optional public detail: the single source of the mapping.
    fn parts(&self) -> (StatusCode, Option<&str>) {
        match self {
            Self::NotFound => (StatusCode::NOT_FOUND, None),
            Self::BadRequest(detail) => (StatusCode::BAD_REQUEST, Some(detail)),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, None),
            Self::Forbidden => (StatusCode::FORBIDDEN, None),
            Self::Conflict(detail) => (StatusCode::CONFLICT, Some(detail)),
            Self::ServiceUnavailable(detail) => (StatusCode::SERVICE_UNAVAILABLE, Some(detail)),
            Self::Internal { .. } => (StatusCode::INTERNAL_SERVER_ERROR, None),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        match &self {
            Self::Internal { source } => tracing::error!(error = %source, "request failed"),
            other if status.is_server_error() => tracing::error!(error = %other, "request failed"),
            other => tracing::warn!(error = %other, "request failed"),
        }
        self.into_problem().into_response()
    }
}

/// Router fallback for unmatched routes: problem+json 404.
pub async fn not_found() -> ApiError {
    ApiError::NotFound
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    async fn body_json(response: Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[test]
    fn problem_serializes_rfc_9457_members() {
        let problem = Problem::new(StatusCode::NOT_FOUND)
            .with_detail("no note with that id")
            .with_instance("/v1/notes/42")
            .with_errors(vec![ProblemError {
                detail: "must be a string".to_owned(),
                pointer: Some("/name".to_owned()),
            }]);

        assert_eq!(
            serde_json::to_value(&problem).unwrap(),
            json!({
                "type": "about:blank",
                "title": "Not Found",
                "status": 404,
                "detail": "no note with that id",
                "instance": "/v1/notes/42",
                "errors": [{"detail": "must be a string", "pointer": "/name"}],
            })
        );
    }

    #[test]
    fn problem_omits_absent_members() {
        let problem = Problem::new(StatusCode::BAD_REQUEST);

        assert_eq!(
            serde_json::to_value(&problem).unwrap(),
            json!({"type": "about:blank", "title": "Bad Request", "status": 400})
        );
    }

    #[tokio::test]
    async fn problem_response_is_problem_json() {
        let response = Problem::new(StatusCode::NOT_FOUND)
            .with_detail("gone")
            .into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            PROBLEM_JSON_CONTENT_TYPE
        );
        assert_eq!(
            body_json(response).await,
            json!({"type": "about:blank", "title": "Not Found", "status": 404, "detail": "gone"})
        );
    }

    #[tokio::test]
    async fn api_error_maps_to_problem_json() {
        let response = ApiError::Conflict("already exists".to_owned()).into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            PROBLEM_JSON_CONTENT_TYPE
        );
        assert_eq!(
            body_json(response).await,
            json!({"type": "about:blank", "title": "Conflict", "status": 409, "detail": "already exists"})
        );
    }

    #[tokio::test]
    async fn internal_error_hides_the_source() {
        let error = ApiError::Internal {
            source: "connection string leaked".into(),
        };

        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_json(error.into_response()).await;
        assert_eq!(body["title"], "Internal Server Error");
        assert_eq!(body["status"], 500);
        assert!(!body.to_string().contains("connection string leaked"));
    }

    #[tokio::test]
    async fn not_found_handler_returns_problem() {
        let response = not_found_handler().await.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            PROBLEM_JSON_CONTENT_TYPE
        );
    }

    async fn not_found_handler() -> ApiError {
        not_found().await
    }

    #[tokio::test]
    async fn not_found_fallback_through_a_router() {
        let router = axum::Router::new().fallback(not_found);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/does-not-exist")
                    .body(Body::empty())
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
}
