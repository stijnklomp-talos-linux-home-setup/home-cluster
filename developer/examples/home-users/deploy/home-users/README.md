# home-users

Helm chart for the `users` reference subgraph: the service, a bundled
Postgres, the `DATABASE_URL` Secret, ambient mTLS, the Cilium network policy,
and an optional test JWKS server.

```sh
helm upgrade --install home-users deploy/home-users \
  --namespace home-users --create-namespace \
  --set image.tag=<tag>
```

The image is pulled from the local registry
(`192.168.1.210:5000/home-users`, HTTP, no TLS). The namespace is labeled
`istio.io/dataplane-mode: ambient` and Pod Security `restricted`.

- **Postgres password**: generated on first install (`randAlphaNum`) and reused
  from the Secret on upgrades; override with `--set postgres.password=...`.
- **GraphQL JWT**: production values point at the Cognito pool. Cognito access
  tokens have no `aud`, so `config.graphqlJwtAudience` is empty and
  `config.graphqlJwtClientId` is the API app client id (replace the
  `<region>`/`<pool>`/`<client-id>` placeholders). For the live JWT-path verification,
  add `-f values-verification.yaml`; it enables the test JWKS
  server and switches to the Cognito access-token shape: issuer
  `https://test-issuer.home.test`, no `aud`, and `client_id:
  home-test-client`. Tokens are signed with the committed test key. The JWKS
  server is test-only; never enable it in production.
- **Probes**: `/healthz` (liveness) and `/readyz` (readiness; 503 while
  Postgres is unreachable).
- **Migrations**: a post-install/post-upgrade Job runs the image's
  `/app/migrate` binary; the init container waits for Postgres.
- **Network policy**: the service accepts the `home-graphql` gateway namespace
  (via ambient HBONE, port 15008), same-namespace callers and host probes;
  egress is DNS, Postgres/JWKS in the namespace, and HTTPS for an external
  JWKS. Direct cross-namespace calls from anywhere else are dropped.
- **Postgres policy**: no separate Cilium policy selects the database. A
  5432-only policy was tried and blocked app/migration traffic (Cilium with
  retained kube-proxy presented the source as the node, not the app endpoint);
  a DB policy is a documented follow-up rather than a service-breaking one.

## Authentication asymmetry

GraphQL is authenticated and authorized (JWT, self-or-`users:read`, verified
email for self-access). The REST surface (`/v1/users`) has no authentication
or ownership check — it is the in-cluster contract and must stay behind the
network policy. Do not expose REST as the public/authenticated path; the
gateway is the only GraphQL entry point and is also ClusterIP-only.

Cognito access tokens do not include `email`/`email_verified` by default. A
pre-token-generation Lambda (or custom-attribute mapping) must add both when
self-email is needed; `custom:user_id` maps the token to `users.id`.
