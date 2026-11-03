//! Validated extractors: serde + garde validation behind axum, with the
//! problem+json error contract.
//!
//! [`axum_valid::Garde`] performs the extraction and validation, but renders its
//! own rejection body. These wrappers delegate to it and translate the
//! rejection into [`Problem`] (ADR-0005): a 400 with the validation details in
//! the `errors` extension array.

use axum::extract::{FromRef, FromRequest, FromRequestParts, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use garde::Validate;
use serde::de::DeserializeOwned;

use crate::problem::{Problem, ProblemError};
use crate::server::AppState;

/// `axum_valid::Garde` reads its validation context from the router state;
/// the DTOs validate with garde's default `()` context.
impl FromRef<AppState> for () {
    fn from_ref(_state: &AppState) -> Self {}
}

/// Extracts and validates a JSON body; an invalid body is a problem+json 400.
pub struct ValidatedJson<T>(pub T);

impl<T, S> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned + Validate<Context = ()>,
    S: Send + Sync,
    (): FromRef<S>,
{
    type Rejection = Problem;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        use axum::extract::Json;
        use axum_valid::{Garde, GardeRejection};

        match Garde::<Json<T>>::from_request(request, state).await {
            Ok(Garde(Json(value))) => Ok(Self(value)),
            Err(GardeRejection::Valid(report)) => Err(rules_to_problem(&report)),
            Err(GardeRejection::Inner(rejection)) => Err(json_rejection_to_problem(rejection)),
        }
    }
}

/// Extracts and validates query parameters; invalid input is a problem+json 400.
pub struct ValidatedQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ValidatedQuery<T>
where
    T: DeserializeOwned + Validate<Context = ()>,
    S: Send + Sync,
    (): FromRef<S>,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        use axum::extract::Query;
        use axum_valid::{Garde, GardeRejection};

        match Garde::<Query<T>>::from_request_parts(parts, state).await {
            Ok(Garde(Query(value))) => Ok(Self(value)),
            Err(GardeRejection::Valid(report)) => Err(rules_to_problem(&report)),
            Err(GardeRejection::Inner(rejection)) => Err(rejection_to_problem(
                rejection.status(),
                rejection.body_text(),
            )),
        }
    }
}

/// Maps a garde report to the problem+json 400 shape: one `errors` entry per
/// failed rule, with the field path as a JSON Pointer.
fn rules_to_problem(report: &garde::Report) -> Problem {
    let errors = report
        .iter()
        .map(|(path, error)| ProblemError {
            detail: error.message().to_owned(),
            pointer: json_pointer(path),
        })
        .collect();

    Problem::new(StatusCode::BAD_REQUEST)
        .with_detail("request validation failed")
        .with_errors(errors)
}

/// Maps a JSON body rejection: axum reports serde data failures (missing
/// fields, wrong types, unknown fields) as 422, but the template's contract is
/// 400 for every invalid body (Fastify parity); a wrong content type stays 415.
fn json_rejection_to_problem(rejection: axum::extract::rejection::JsonRejection) -> Problem {
    let status = if rejection.status() == StatusCode::UNSUPPORTED_MEDIA_TYPE {
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    } else {
        StatusCode::BAD_REQUEST
    };
    rejection_to_problem(status, rejection.body_text())
}

/// Maps an inner-extractor rejection (malformed JSON, missing fields, bad
/// query string, wrong content type) to the same shape.
fn rejection_to_problem(status: StatusCode, detail: String) -> Problem {
    Problem::new(status)
        .with_detail("request validation failed")
        .with_errors(vec![ProblemError {
            detail,
            pointer: None,
        }])
}

/// garde paths render as `field`, `field.subfield` or `field[0]`; JSON Pointer
/// uses `/` separators and leaves array indices as-is (`/field/0` mapping is
/// done only for `.`-separated struct paths).
fn json_pointer(path: &garde::Path) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    Some(format!("/{}", path.to_string().replace('.', "/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal validated payload so these tests stay independent of any slice.
    #[derive(Debug, garde::Validate)]
    struct Payload {
        #[garde(length(chars, max = 3))]
        name: String,
    }

    #[test]
    fn validation_failures_become_a_400_problem_with_pointers() {
        let report = Payload {
            name: "four".to_owned(),
        }
        .validate()
        .unwrap_err();

        let problem = rules_to_problem(&report);

        assert_eq!(problem.status, 400);
        assert_eq!(problem.title, "Bad Request");
        let errors = problem.errors.expect("errors extension");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].pointer.as_deref(), Some("/name"));
        assert!(!errors[0].detail.is_empty());
    }

    #[test]
    fn empty_paths_have_no_pointer() {
        let mut report = garde::Report::new();
        report.append(garde::Path::empty(), garde::Error::new("bad request"));

        let problem = rules_to_problem(&report);

        assert_eq!(problem.errors.unwrap()[0].pointer, None);
    }

    #[test]
    fn rejections_become_a_problem_with_a_single_error() {
        let problem = rejection_to_problem(StatusCode::BAD_REQUEST, "missing field".to_owned());

        assert_eq!(problem.status, 400);
        let errors = problem.errors.expect("errors extension");
        assert_eq!(errors[0].detail, "missing field");
    }
}
