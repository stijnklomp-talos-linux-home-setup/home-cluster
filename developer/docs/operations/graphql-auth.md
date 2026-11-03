# GraphQL JWT authentication and authorization

How a Cognito token reaches Hive Gateway and the two Rust subgraphs, which
claims are trusted, and where each authorization decision is made. Status:
implemented 2026-11-01 (ticket #17); the interactive Cognito sign-in is the
only manual step (there are no Cognito credentials in this environment).

> Public document: use placeholders such as `<lb-ip>` and `<cognito-pool-id>`.
> The real cluster addresses and pool ids live in the cluster configuration.

## Status

| Check | Result (2026-11-01) |
|---|---|
| Gateway validates RS256 signature + issuer from a JWKS | yes |
| Missing / invalid / malformed token before query planning | **401**, no subgraph request |
| Raw `Authorization` forwarded to both subgraphs | yes; each service validates again |
| Coarse scopes enforced by Hive (`@requiresScopes`) | `users:read`, `orders:read`, `orders:admin` |
| Field/owner rules in the Rust services | self (verified email) or `users:read`; own orders or `orders:admin` |
| Self-email without `users:read` | own verified `email` (200); another user's email denied |
| Trusted identity headers (`X-User`/`X-Email`) | never read; spoofing has no effect |
| Direct subgraph access from an unrelated ambient namespace | Cilium drops it |
| Direct subgraph access from the gateway namespace | valid token 200, no token 401 |
| Real Cognito access token end to end | **manual** (interactive login) |

The live deployment uses the committed verification overlay: the test issuer
`https://test-issuer.home.test`, the in-cluster test JWKS, no `aud`, and
`client_id: home-test-client`. That is the same claim shape as a Cognito
*access* token, so the Cognito code path is exercised without Cognito
credentials. Production values keep `<region>`/`<pool>`/`<client-id>`
placeholders; a dedicated GraphQL app client is still pending, so the live
deployment stays on the verification overlay.

## Token flow

```
caller (UI / in-cluster client)
  │  Authorization: Bearer <access token>
  ▼
Hive Gateway (ClusterIP only, ambient)
  │  1. JWT plugin: RS256 signature via JWKS, issuer, exp; `client_id` match
  │  2. genericAuth: @requiresScopes for the selected fields
  │  3. raw Authorization header is propagated (no payload/identity headers)
  ▼
home-users /graphql          home-orders /graphql
  │  4. each service verifies the same token against the JWKS
  │  5. resolver guards: self / scope / owner
  ▼
Postgres
```

The gateway has no edge route: the Next.js UI or another in-cluster caller
reaches it over the Service DNS name. Ambient ztunnel encrypts every hop
(`PeerAuthentication: STRICT` in `home-graphql`, `home-users` and
`home-orders`); Cilium admits only the gateway namespace into the subgraph
service ports.

## Claims

Cognito **access** tokens are what the API consumes. They carry:

| Claim | Use |
|---|---|
| `iss` | Required; must equal `GRAPHQL_JWT_ISSUER` / `JWT_ISSUER` |
| `exp` | Required; validated with the usual clock leeway |
| `client_id` | Required when configured; the app client that requested the token |
| `scope` | Space-delimited scopes (`users:read orders:read ...`) |
| `sub` | Caller subject (informational; Cognito's is a UUID) |
| `username` | Fallback for the caller's email when the pool username is an address |
| `email` | Not in Cognito access tokens by default; add it with a pre-token Lambda if needed |
| `email_verified` | Must be `true` for `email` to identify the caller for self-access |
| `custom:user_id` | Optional local user id claim; the clean way to map Cognito users to `users.id` |

**No `aud`.** Access tokens have no audience claim; only ID tokens do. Setting
`JWT_AUDIENCE`/`GRAPHQL_JWT_AUDIENCE` to a non-empty value switches to `aud`
validation (used by the older verification overlay and tests). For Cognito
access tokens leave the audience empty and set the client id instead.

The local user id for ownership checks comes from `user_id`,
`custom:user_id` (Cognito custom attribute) or a numeric `sub`, in that order.
A real Cognito `sub` is a UUID, so the pool must emit `custom:user_id` (via a
pre-token-generation Lambda or a custom attribute) for the orders ownership
rules to identify the caller. The users service can also identify the caller by
`email` (or a username that contains `@`) **only when `email_verified` is
true**; without one of these claims the service denies self/owner access rather
than guessing.

## Cognito setup (manual)

1. Create or reuse an **API app client** in the user pool (no client secret for
   a public client; a confidential client is fine for a server-side caller).
   Its client id is the `JWT_CLIENT_ID` / `GRAPHQL_JWT_CLIENT_ID` value for the
   gateway and both services. The SPA/UI client is only usable if it is the
   same client id.
2. Define the resource-server scopes `users:read`, `orders:read` and
   `orders:admin`, and allow the app client to request them.
3. Add a pre-token-generation Lambda (or a custom attribute mapping) that puts
   `custom:user_id` (the local `users.id`) and, if you rely on email, both
   `email` and `email_verified: true` into the **access** token. Cognito does
   not copy either claim into access tokens by default.
4. Point `graphqlJwksUrl` / `graphqlJwtIssuer` / `graphqlJwtClientId` at the
   pool and client; replace the `<region>`/`<pool>`/`<client-id>` placeholders
   in all three charts. The gateway refuses to start while the issuer still
   contains a placeholder.
5. Verify with a real interactive login: sign in through the hosted UI, take
   the **access token** (not the ID token), and call the gateway with
   `Authorization: Bearer <access token>`. Confirm `users:read` returns
   another user's `User.email`; without it the caller still sees their own
   verified email, another user's email is denied, and another user's order is
   null.

That interactive step has not been executed here; the environment has no
Cognito credentials or browser. The rest of the path is verified with the
test issuer.

## Gateway rules

- `jwt.reject.missingToken` and `invalidToken` are on. Validation runs in the
  request-parse phase, before query planning or any subgraph request.
- A custom bearer lookup normalizes every unusable header (not a JWT, empty
  bearer, wrong prefix) to **401**; the Yoga plugin would otherwise return 400
  for an undecodable value.
- A token minted for another app client is rejected with 401 even if its
  signature and issuer are valid.
- `genericAuth.mode: protect-granular` plus `resolveUserFn`/`extractScopes`
  enforces Federation `@requiresScopes`; the default `scope` extraction splits
  the space-delimited claim, and an `scp` array is accepted too.
  `rejectUnauthenticated: true` makes an insufficient scope a 401 for the whole
  operation rather than a partial 200.
- `User.email` deliberately has **no** `@requiresScopes`: Hive cannot express
  "self or scope", so the users resolver is the guard and the field is allowed
  through for a valid token. The coarse list query (`users`) still carries
  `@requiresScopes(users:read)`.
- Only the raw `Authorization` header is propagated. The parsed payload is not
  forwarded, and no `X-User`/`X-Email` header is produced or trusted.

## Service rules

| Field / query | Rule |
|---|---|
| `viewer` | Always the caller: `subject`, `email`, `emailVerified`, `userId`, `scopes` |
| `users(page, perPage)` | `users:read` (`@requiresScopes` + resolver check) |
| `User.email` | Self with a verified email, or `users:read`; Hive does not gate the field, the resolver does |
| `user(id:)` | Self with a verified email or `users:read`; otherwise `null` (no existence leak) |
| `_entities` `User` | Name resolvable; `email` still guarded by self/`users:read` |
| `orders(page, perPage)` | `orders:read` or `orders:admin`; non-admin filtered to the caller |
| `order(id:)` / `_entities` `Order` | Own order or `orders:admin`; otherwise `null` |

A direct subgraph call is protected by the same code, not only by the gateway:
the service validates the JWT again and the resolver guard makes the same
decision.

## Live verification (test issuer)

Evidence captured on 2026-11-01 against the verification overlay. Tokens were
minted by `developer/examples/home-graphql/scripts/mint-test-jwt.mjs` with no `aud`,
`client_id: home-test-client`, `sub`, `email` and `scope`.

| Request | Result |
|---|---|
| `users:read orders:read`, user 1 | 200; user 1 data, orders 1–2, cross-service `user.name/email` |
| no `users:read`, `users { id email }` | 401 gateway (`UNAUTHORIZED_FIELD_OR_TYPE`) |
| no `users:read`, `viewer.email` / `user(id: self).email` | 200; caller's verified email |
| no `users:read`, another user's entity `email` | resolver denies (`users:read` required) |
| no `users:read`, `orders { user { name } }` | 200; no email field requested |
| no `orders:read`/`orders:admin`, `orders` | 401 gateway |
| user 1, `order(id: <user 2 order>)` | 200 `{ "order": null }` |
| `orders:admin` | 200; every owner's orders |
| wrong `client_id` | 401 gateway |
| missing token | 401 |
| malformed/invalid token | 401 |
| `X-User`/`X-Email` for user 2, no token | 401 |
| valid user 1 token + spoofed user 2 headers | 200; still user 1 |
| invalid or insufficient-scope request | no subgraph `/graphql` request (log delta 0) |
| direct subgraph from `ambient-sandbox` | Cilium drop (timeout) |
| direct subgraph from `home-graphql` with token | 200 |
| direct subgraph from `home-graphql` without token | 401 problem+json + `WWW-Authenticate: Bearer` |
| direct subgraph malformed JSON body (valid token) | 400 problem+json |

## Switching to production Cognito

1. Create a dedicated GraphQL API app client (or deliberately reuse an
   existing one) and fill in the `<region>`, `<pool>` and `<client-id>`
   placeholders in `home-users`, `home-orders` and `home-graphql` values.
   No pool id or client id is committed.
2. Deploy without `values-verification.yaml`; the test JWKS server is disabled
   and the gateway/service fetch the Cognito JWKS over HTTPS (the gateway
   Cilium policy allows the JWKS host from `config.jwksUri` over 443 through a
   DNS L7 rule).
3. In `home-cluster`, set `valuesFile: ""` for the three GraphQL declarations;
   they currently point at `values-verification.yaml`. Those declarations are
   single-environment (the charts, supergraph and CNPs use fixed namespaces),
   so do not expect a real dev/staging/prod rollout.
4. Run the manual interactive sign-in above and record the result.

The verification overlay stays committed so the JWT path can be re-proven
without Cognito; it must not be used for real users.
