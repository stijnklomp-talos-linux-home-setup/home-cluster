# OpenAPI is always generated from code

DTOs are single structs carrying `serde` (serialization), `utoipa::ToSchema` (documentation) and `garde::Validate` (runtime validation via axum-valid). The OpenAPI 3.1 document is generated from the code — written on non-production startup and by `just openapi` — and never hand-edited. `APP_ENV` replaces `NODE_ENV`; no Swagger UI ships (matching the Fastify template).

Consequence: validation constraints appear twice, in `#[garde]` (runtime) and `#[schema]` (documentation). The small duplication is accepted until an integration like `vld-utoipa` matures.
