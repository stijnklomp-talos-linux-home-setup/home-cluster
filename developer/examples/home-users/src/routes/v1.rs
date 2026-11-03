//! The `v1` route surface.

pub mod users;

use utoipa_axum::router::OpenApiRouter;

use crate::server::AppState;

/// Routes under `/v1` (each handler's `#[utoipa::path]` carries the absolute path).
pub fn routes() -> OpenApiRouter<AppState> {
    users::routes()
}
