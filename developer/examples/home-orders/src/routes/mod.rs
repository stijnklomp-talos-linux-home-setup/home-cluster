//! Route assembly: the axum router and the generated OpenAPI document are
//! built from the same source (`app_router`), so they cannot drift.

pub mod v1;

use utoipa_axum::router::OpenApiRouter;

use crate::server::AppState;

/// Every route the app serves (health + versioned surfaces).
pub fn app_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(crate::health::routes())
        .merge(v1::routes())
}
