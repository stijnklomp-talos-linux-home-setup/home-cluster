//! Users routes: registers the users controllers with axum and feeds their
//! `#[utoipa::path]` documentation into the generated OpenAPI document.

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

// utoipa generates one `__path_<handler>` item per annotated handler; the
// `routes!` macro resolves them by name, so both must be in scope here.
use crate::controllers::users::{__path_get_user, __path_get_users, get_user, get_users};
use crate::server::AppState;

/// `GET /v1/users` and `GET /v1/users/{id}`.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_users))
        .routes(routes!(get_user))
}
