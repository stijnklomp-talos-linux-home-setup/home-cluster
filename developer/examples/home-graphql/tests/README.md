# Tests and checks

`tests/fixtures/graphql-test-key.pem` is a throwaway RSA key used by the test
JWTs and served by the in-cluster test JWKS. It has no value outside this repo.

The gateway repo has no runtime test harness. The checks are:

```sh
# Hive config syntax (it throws on purpose when JWT_ISSUER is unset, so this
# only parses the module; it does not evaluate the config).
docker run --rm -v "$PWD:/src:ro" -w /src node:22-alpine node --check gateway.config.mjs
docker run --rm -v "$PWD:/src:ro" -w /src node:22-alpine node --check scripts/mint-test-jwt.mjs

# The chart must ship the same config/supergraph as the repository root.
./scripts/check-config-sync.sh

# Chart rendering with both overlays.
docker run --rm -v "$PWD:/src" -w /src alpine/helm:latest lint deploy/home-graphql
docker run --rm -v "$PWD:/src" -w /src alpine/helm:latest template home-graphql deploy/home-graphql -n home-graphql -f deploy/home-graphql/values-verification.yaml
```

The live matrix (token shapes, 401s, no subgraph call for rejected tokens,
cross-service query) is run against the cluster; see `../README.md` and
`developer/docs/operations/graphql-auth.md`.
