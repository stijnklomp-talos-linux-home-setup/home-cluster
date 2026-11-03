# Optional GraphQL federation subgraph

Status: Accepted (2026-11-01)

Ticket #13 of the service-networking epic. The platform decision is in the wiki ADR `service-networking-adr.md`; this ADR records only how the Rust template implements it.

## Context

Every service keeps its REST contract (OpenAPI + Pact). GraphQL is opt-in and, when enabled, the service is an Apollo Federation v2 subgraph. The template must not fork: the default build stays REST-only, and one Cargo feature adds the subgraph.

## Decision

- **Feature and dependencies.** `graphql` is an optional Cargo feature pulling `async-graphql` 7.2.1 (stable; 8.0.0 was still a release candidate) and `async-graphql-axum`, `jsonwebtoken` 11.1 (JWT/JWKS, `aws_lc_rs` backend), `reqwest` and `rustls` (JWKS fetch). The default build compiles none of them.
- **Route.** `GET`/`POST /graphql`, merged into the app router only when the feature is compiled and `GRAPHQL_ENABLED` is not `false`. It is deliberately absent from `openapi.yaml`: that document is the REST contract.
- **Federation.** The sample `User` entity is declared by a `#[graphql(entity)]` resolver on `Query`, so async-graphql emits `type User @key(fields: "id")`, `_service { sdl }` and `_entities`. The SDL links `https://specs.apollo.dev/federation/v2.5`.
- **Batching.** Entity resolution goes through `async_graphql::dataloader::DataLoader`: one loader per GraphQL request batches every representation in a `_entities` call into a single repository lookup. `OrderLoader` in `src/graphql/schema.rs` is the template for a real repository-backed loader.
- **Authentication.** Every GraphQL request needs `Authorization: Bearer <JWT>`. `JwtVerifier` accepts RS256 only, requires `exp` and `iss`, and validates the issuer plus the configured audience **or** Cognito `client_id`. `GRAPHQL_JWT_AUDIENCE` may be empty because Cognito access tokens have no `aud`; `GRAPHQL_JWT_CLIENT_ID` then supplies the equivalent check. The JWKS is fetched lazily on the first request (an unavailable IdP cannot block boot) and refreshed when a token carries an unknown `kid`. The validated caller is inserted into the GraphQL context as `AuthContext` (`sub`, `email`, `user_id`, scopes).

  *Amended by ticket #17 (2026-11-01):* the service also emits Federation `@requiresScopes` for the coarse checks (`users:read`, `orders:read`, `orders:admin`) and enforces self/owner rules in the resolvers; Hive enforces the same directives. `@requiresScopes` authorization is no longer out of scope.
- **Failure behavior.** Missing or invalid tokens are problem+json **401** before GraphQL executes; a JWKS fetch failure is a problem+json **503**. Internal validation details are logged, never returned.
- **Configuration.** `GRAPHQL_ENABLED` (default `true` when compiled), `GRAPHQL_JWKS_URL`, `GRAPHQL_JWT_ISSUER`, `GRAPHQL_JWT_AUDIENCE` (default: the package name; empty disables the `aud` check for Cognito access tokens) and `GRAPHQL_JWT_CLIENT_ID` (required `client_id` claim, the Cognito access-token equivalent of `aud`). The defaults point at a local Keycloak realm; deployments set real values. No secret is stored in the repository.

## Consequences

- **Easier:** one template serves REST-only and federated services; the subgraph follows the same config/logging/problem+json conventions; JWT validation is enforced locally even though Hive Gateway also validates.
- **Harder:** GraphQL adds a second API shape with its own tests and auth path; RS256-only is a deliberate constraint for the current IdPs (Keycloak, Cognito), and another algorithm needs a code change; `async-graphql` 8 will be a deliberate upgrade once stable.
- **Out of scope:** schema composition (`home-graphql`/rover), subscriptions, GraphiQL, and exposing `/graphql` in OpenAPI. Federation `@requiresScopes` and field/owner rules are implemented; see the amendment above.
