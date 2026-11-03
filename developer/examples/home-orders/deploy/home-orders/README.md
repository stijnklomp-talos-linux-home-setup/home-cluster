# home-orders

Helm chart for the `orders` subgraph: the service, a bundled Postgres on
Longhorn, the generated credentials Secret, the migration Job, the ambient
namespace and its policy.

```sh
helm upgrade --install home-orders deploy/home-orders \
  --namespace home-orders --create-namespace
```

The default image is the in-cluster registry
(`192.168.1.210:5000/home-orders`, HTTP, no TLS). The namespace is labeled
`istio.io/dataplane-mode: ambient` and Pod Security `restricted`; the
`PeerAuthentication` is `STRICT`, so every hop (including Postgres) is mTLS.

## Test JWKS / JWT verification

Production values point at the Cognito pool. Cognito access tokens have no
`aud`, so `config.graphqlJwtAudience` is empty and `config.graphqlJwtClientId`
is the API app client id (replace the `<region>`/`<pool>`/`<client-id>`
placeholders). For the live JWT-path verification, add
`-f values-verification.yaml`; it enables the bundled test
JWKS server and switches to the Cognito access-token shape: issuer
`https://test-issuer.home.test`, no `aud`, and `client_id: home-test-client`.
The JWKS server is test-only; never enable it in production.

Authorization: `orders`/`order(id:)` require `orders:read` or `orders:admin`
(`@requiresScopes` in the SDL). Without `orders:admin` the list is filtered to
the caller's `userId` and another owner's order resolves to null.

## Policy

The chart's `CiliumNetworkPolicy` admits ingress only from the `home-graphql`
namespace (the gateway) and host/health probes. Egress is DNS, Postgres and the
in-namespace JWKS (plus HTTPS for an external JWKS). Redis/RabbitMQ are disabled
in the config; orders has no cache or broker dependency. Postgres has no
separate Cilium policy: a 5432-only policy was tried and blocked
app/migration traffic (Cilium with retained kube-proxy presented the source as
the node, not the app endpoint), so it is a documented follow-up rather than a
service-breaking policy.

## Migrations

The `migrate` Job runs as a post-install/post-upgrade hook with the same image
(`/app/migrate`), so a deploy never serves against an unmigrated schema. Seed
the table with `just db-seed` against a dev database.

## `custom:user_id` (required for ownership)

Non-admin callers only see their own orders. Orders has no users database, so
the ownership key comes from the token: `custom:user_id` (preferred),
`user_id`, or a numeric `sub`. A real Cognito `sub` is a UUID, so the user
pool must emit `custom:user_id` (the local `users.id`) in the **access** token
via a pre-token-generation Lambda or a custom-attribute mapping. Without it a
non-admin list returns an error and a single order resolves to `null`.

## Authentication asymmetry

GraphQL is authenticated and owner-filtered; REST (`/v1/orders`) has no
authentication or ownership check. It is the in-cluster contract and must stay
behind the network policy — never expose REST as the authenticated path.
