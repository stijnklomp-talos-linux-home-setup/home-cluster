# home-graphql

The GraphQL **aggregation path** for the home cluster: a [Hive Gateway
2.15.x](https://the-guild.dev/graphql/hive/docs/gateway) serving the
Rover-composed supergraph of the `users` and `orders` subgraphs. REST/OpenAPI
remains the direct service contract; this gateway is ClusterIP-only and has
no edge route.

One replica, no PDB: at this scale a PDB cannot provide availability with one
replica and would only block node drains (see the service-networking ADR in the
wiki repo).

## Layout

| Path | Purpose |
|---|---|
| `supergraph.yaml` | Rover config: subgraph routing URL + `federation_version: =2.9.3` |
| `schemas/users.graphql` | The users subgraph SDL, fetched verbatim from `_service { sdl }` |
| `schemas/orders.graphql` | The orders subgraph SDL, fetched verbatim from `_service { sdl }` |
| `supergraph.graphql` | Composed artifact, committed; the chart serves it from a ConfigMap |
| `gateway.config.mjs` | Hive config (JS only — Hive has no YAML config) |
| `deploy/home-graphql` | Helm chart: Deployment, ClusterIP Service, ConfigMaps, probes, ambient namespace, STRICT mTLS, Cilium policy |
| `scripts/compose.sh` | Docker-only `rover supergraph compose` + sync into the chart |
| `scripts/check-config-sync.sh` | Asserts the chart copies match the canonical root files |
| `scripts/mint-test-jwt.mjs` | Test JWT for live verification (RS256, `kid: test-key`) |
| `tests/fixtures/graphql-test-key.pem` | Throwaway test key, the same fixture home-users tests use |
| `tests/README.md` | The offline checks (syntax, sync, chart render) |

`deploy/home-graphql/files/` holds copies of the gateway config and supergraph
because Helm `.Files` cannot read outside the chart. `scripts/compose.sh` writes
both copies; never edit the chart copies by hand.

## Compose

Rover is pinned to 1.0.1 with the ELv2 `supergraph` plugin 2.9.3. Everything
runs through Docker; the plugin is cached in the `home-graphql-rover` volume.

```sh
./scripts/compose.sh
```

The subgraph routing URLs are the in-cluster Service DNS names
(`http://home-users.home-users.svc.cluster.local/graphql` and
`http://home-orders.home-orders.svc.cluster.local/graphql`), so Hive reaches
them through ambient mTLS. To refresh an SDL after a schema change, query the
live subgraph with a valid token and save `data._service.sdl` verbatim:

```sh
kubectl -n home-users exec deploy/home-users-caller -- \
  curl -sS -X POST http://developer/examples/home-users/graphql \
    -H 'content-type: application/json' \
    -H "authorization: Bearer $TOKEN" \
    -d '{"query":"{ _service { sdl } }"}' | jq -r '.data._service.sdl' > schemas/users.graphql

# orders: from the gateway namespace, the only caller its policy admits
kubectl -n home-graphql exec deploy/home-graphql-caller -- \
  curl -sS -X POST http://home-orders.home-orders.svc.cluster.local/graphql \
    -H 'content-type: application/json' \
    -H "authorization: Bearer $TOKEN" \
    -d '{"query":"{ _service { sdl } }"}' | jq -r '.data._service.sdl' > schemas/orders.graphql
```

## Schema changes reach the gateway without a rebuild

The chart's `home-graphql-supergraph` ConfigMap is the runtime schema source:
the Deployment mounts it as a **directory** and `gateway.config.mjs` reads it
through a function source, so Hive polls the file every 10 s and picks up the
kubelet's atomic symlink swap. A schema change is therefore a ConfigMap
update — not a gateway image build and not a pod restart.

Publish a schema change with the shared Tekton pipeline (manual; **no
breaking-change gate** in this iteration):

```sh
kubectl create -f ../developer/pipelines/charts/supergraph-release/usage-examples/v1/pipelinerun.yaml
```

`supergraph-release-v1` runs `supergraph-compose-v1` (Rover 1.0.1,
`supergraph@=2.9.3`, `--skip-update`, composes into a temp file and moves the
artifact only on success) and then `supergraph-configmap-v1` (`kubectl create
configmap --dry-run=client | kubectl apply`, then reads the object back and
compares the content hash). The chart renders the `supergraph-release`
ServiceAccount/Role/RoleBinding in `home-graphql`; the Role only allows
`get`/`update`/`patch` on that one ConfigMap (plus `create` in the namespace).

Failure behaviour: an invalid SDL or a broken federation reference fails the
compose Task; `publish-configmap` never runs, the ConfigMap is untouched and
Hive keeps serving the last good supergraph. Verify with
`kubectl -n home-graphql get pods` (restart count unchanged) and a query for
the new field. Propagation is the kubelet's ConfigMap sync (up to ~1 min)
plus the 10 s poll.

After a successful release, commit the refreshed `schemas/*.graphql` and
`supergraph.graphql` (see below): a later `helm upgrade` ships the committed
artifact and would otherwise roll the ConfigMap back.

## Deploy

```sh
helm upgrade --install home-graphql deploy/home-graphql \
  --namespace home-graphql --create-namespace
```

The image is `ghcr.io/graphql-hive/gateway:2.15.4` (public). The deployment
passes `supergraph -c /gateway/config/gateway.config.mjs` as args because the
image has no CMD. Both ConfigMaps are mounted as **directories** (never
`subPath`); the config reads the supergraph through a function source and polls
every 10 s, so a schema change is a ConfigMap update, not a pod restart.

`values.yaml` is the production default: the Cognito issuer/JWKS use
`<region>`/`<pool>`/`<client-id>` placeholders, `jwtAudience` is empty (access
tokens have no `aud`) and `jwtClientId` is the API app client id. The gateway
**fails at startup** while `JWT_ISSUER` is unset or still contains a
placeholder, so a forgotten value cannot silently validate against the wrong
pool. The live deployment applies `values-verification.yaml`, which points at
the in-cluster test JWKS and uses `client_id: home-test-client` so the Cognito
access-token shape is exercised end to end. Switch back by replacing the
placeholders and dropping that values file.

`home-cluster` declares this service in `developer/apps/baremetal/home-graphql.yaml`; that
declaration owns the image tag in GitOps.

## Verify

Mint a test token (issuer `https://test-issuer.home.test`, key
`tests/fixtures/graphql-test-key.pem`). The verification deployment uses the
Cognito access-token shape: no `aud`, `client_id: home-test-client`, and a
space-delimited `scope` claim. Tokens for the older audience mode carry
`home-users` for the gateway/users and `home-orders` for orders.

```sh
TOKEN=$(docker run --rm -v "$PWD:/src:ro" -w /src node:22-alpine \
  node scripts/mint-test-jwt.mjs --aud "" --client-id home-test-client \
    --sub 1 --email ada@example.test --scope "users:read orders:read")

# audience mode (previous tickets): --aud home-users,home-orders
```

From a caller in the `home-graphql` namespace (the only namespace the Cilium
policy admits, besides host probes):

```sh
curl -sS http://home-graphql.developer/examples/home-graphql/graphql \
  -H 'content-type: application/json' \
  -H "authorization: Bearer $TOKEN" \
  -d '{"query":"{ viewer { subject email userId } users { id name email } }"}'

# the reference cross-service query: order fields from orders, user fields
# from users
curl -sS http://home-graphql.developer/examples/home-graphql/graphql \
  -H 'content-type: application/json' \
  -H "authorization: Bearer $TOKEN" \
  -d '{"query":"{ orders { id total status user { id name email } } }"}'
```

Denials are **401** before query planning: missing token, bad signature,
wrong issuer, wrong `client_id`, and a malformed/non-JWT bearer value (the
custom lookup normalizes the plugin's 400 to 401). With a valid token, a
missing `@requiresScopes` grant (for example `users:read` on the `users`
list) is also a 401 from `genericAuth`; no subgraph request is made.
`User.email` has no directive because "self or scope" is not expressible in
Hive: the users resolver serves the caller's own verified email without
`users:read` and denies everyone else. A field/owner denial inside a subgraph
is a normal GraphQL `null` for
the denied node (for example another user's `order(id:)`). `/healthcheck`
(liveness) and `/readiness` (ready only when a supergraph is loaded) are on
port 4000.

## Security model

- Namespace `home-graphql` is ambient (`istio.io/dataplane-mode: ambient`) and
  Pod Security `restricted`; `PeerAuthentication` is `STRICT`.
- The Cilium policy admits ingress only from this namespace and host/health
  probes; egress is DNS plus the configured backend namespaces (`home-users`,
  `home-orders` and the test JWKS) over HBONE.
- The gateway validates the JWT (RS256, issuer, expiry, and `aud` or the
  Cognito `client_id`) and enforces Federation `@requiresScopes` through
  `genericAuth` (`protect-granular`). It forwards the raw `Authorization`
  header only; the subgraph validates it again and applies the field/owner
  rules. No parsed-payload or trusted identity headers cross the boundary.
- The production defaults are Cognito placeholders (no pool id is committed)
  with an empty audience; the gateway refuses to start until the issuer is
  replaced. The live verification overlay uses the test issuer/JWKS and
  `client_id: home-test-client`; the test key is public and test-only.
- The Cilium policy's DNS egress carries an L7 `matchPattern: "*"` rule and the
  Cognito FQDN is derived from `config.jwksUri`: Cilium only populates the
  FQDN cache from DNS responses that pass through its proxy. See `developer/docs/operations/graphql-auth.md`.

## Reference federation example

`users` owns `User` (`@key(fields: "id")`). `orders` owns `Order` and declares
the minimal reference `type User @key(fields: "id", resolvable: false)
{ id: ID! }`; `Order.user` returns only
`{ __typename: "User", id: userId }`.

```graphql
{ orders { id total status user { id name email } } }
```

The query planner asks `orders` for the order rows (one list query), then sends
**one** `_entities` request with every user representation in that page to
`users`. The users service's `UserLoader` turns that into one
`WHERE id = ANY($1)` database query, so N orders do not produce N user
requests. The subgraph request logs (one POST per subgraph per query) and the
users `_entities` payload are the batching evidence.

Both subgraphs validate the forwarded bearer token independently; the gateway
never injects identity headers. Direct calls to either subgraph from an
ambient sandbox namespace are denied by Cilium, and both namespaces are
`STRICT` mTLS, so only this gateway namespace reaches them, over HBONE.
