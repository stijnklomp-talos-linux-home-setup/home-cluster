//! Orders routes: registers the orders controllers with axum and feeds their
//! `#[utoipa::path]` documentation into the generated OpenAPI document.

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

// utoipa generates one `__path_<handler>` item per annotated handler; the
// `routes!` macro resolves them by name, so both must be in scope here.
use crate::controllers::orders::{__path_get_order, __path_get_orders, get_order, get_orders};
use crate::server::AppState;

/// `GET /v1/orders` and `GET /v1/orders/{id}`.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_orders))
        .routes(routes!(get_order))
}
