# supergraph-release

Composes the home supergraph with Apollo Rover and publishes it as the Hive
Gateway's supergraph ConfigMap. A schema change reaches the gateway as a
ConfigMap update that Hive polls every 10 s — no gateway image rebuild, no pod
restart. End-to-end, from PipelineRun start to the new schema being served,
was measured at **12–57 s** (budget about a minute); the 10 s figure is only
Hive's poll interval.

| Object | Namespace | Purpose |
|---|---|---|
| `Task/supergraph-seed-v1` | `tekton-pipelines` | unpacks the `source.tgz.b64` ConfigMap into the `source` PVC (no git remote for `home-graphql`) |
| `Task/supergraph-compose-v1` | `tekton-pipelines` | `rover supergraph compose` from `supergraph.yaml` + `schemas/`; pinned Rover 1.0.1 / `supergraph@=2.9.3` / `--skip-update`; deletes any stale artifact, writes a temp file, moves it on success only |
| `Task/supergraph-configmap-v1` | `tekton-pipelines` | renders `kubectl create configmap --dry-run=client -o yaml` to a file, `kubectl apply`s it, stamps provenance and compares the read-back hash |
| `Pipeline/supergraph-release-v1` | `tekton-pipelines` | clone *or* seed → compose → publish |
| `ServiceAccount` + `Role` + `RoleBinding` `supergraph-release` | `home-graphql` (values) | `get`/`update`/`patch` on the one ConfigMap, **no** `create` |
| `PersistentVolumeClaim/supergraph-release-source` | `home-graphql` (values) | seeded checkout + composed artifact + Rover plugin cache (`source.pvc.create`) |

The RBAC and the PVC are the only namespaced objects. They render into the
namespace that owns the ConfigMap (`configMap.namespace`, default
`home-graphql`), not in `tekton-pipelines`, and never into a tenant.

## Ordering

Install the gateway chart **first**, then this catalog release:

1. `home-graphql` creates the namespace, the `home-graphql-supergraph`
   ConfigMap and the ArgoCD Application.
2. This chart's RBAC/PVC render into that existing namespace (`helmDefaults`
   has `createNamespace: false`).
3. The publisher only updates an existing ConfigMap; it cannot create one, so
   publishing before step 1 fails with an explicit "does not exist" message.

For an `<app>-<env>` install (e.g. `home-graphql-dev`), install this release
with matching values: `configMap.namespace=home-graphql-dev`, the ConfigMap name
that environment renders, and `source.pvc.name` unique per environment. Change
`config-map-namespace` in the PipelineRun too; do not reuse the prod PVC.

## Usage

`home-graphql` lives only in the working tree (its ArgoCD declaration points at
a GitHub repo that is not reachable from this cluster), so the documented path
seeds the PVC from a ConfigMap. Run this from the `home-graphql` checkout once
per schema change:

```sh
# gzip tar of the composition inputs, base64 because ConfigMap data is UTF-8
tar -czf - supergraph.yaml schemas \
  | base64 | tr -d '\n' > /tmp/supergraph-source.b64

kubectl -n home-graphql create configmap supergraph-source \
  --from-file=source.tgz.b64=/tmp/supergraph-source.b64 \
  --dry-run=client -o yaml | kubectl apply -f -
rm -f /tmp/supergraph-source.b64
```

Then trigger the release:

```sh
kubectl create -f charts/supergraph-release/usage-examples/v1/pipelinerun.yaml
kubectl -n home-graphql get pipelinerun -w
```

The seed ConfigMap must be in the PipelineRun namespace (the gateway
namespace). It is created by hand and deliberately carries no Helm/ArgoCD
labels, so neither tool prunes it. Keep it under 1 MiB (the checkout is a few
KB). `supergraph-seed-v1` replaces the checkout in the PVC on every run and
preserves `.rover-home/`, the plugin cache.

Alternatively, set `repo-url` (and optionally `revision`/`subdirectory`) to
clone with `git-clone-v1`; the seed Task is then skipped. `source` must be a
PVC either way: compose and publish are separate TaskRuns, the composed
artifact is handed over through the workspace, and the plugin cache lives
there. The example binds `claimName: supergraph-release-source`; a per-run
`volumeClaimTemplate` starts empty and cannot hold the seed or the cache.

`config-map-name`/`config-map-namespace` default to the chart values. The
PipelineRun needs `serviceAccountName: supergraph-release` (the RBAC above) and
`podTemplate.securityContext.fsGroup: 65532` so UID 65532 can write the PVC.

## GitOps deviation

**Read this before running a release.** The publish Task writes
`home-graphql-supergraph` directly, but the same ConfigMap is rendered by the
`home-graphql` Helm chart, which ArgoCD syncs. Immediately after a release the
Application is **OutOfSync**, and a human Sync would revert the ConfigMap to
the committed `files/supergraph.graphql`.

This is deliberate and documented, not an oversight: `home-graphql` has no
reachable remote, so the release cannot bump a GitOps-managed value/file
itself (the usual `gitops-yaml-bump-v1` path needs a pushable repo). The
publisher stamps `supergraph.home/released-at`, `supergraph.home/released-by`
and `supergraph.home/sha256` annotations so the drift is identifiable.

Reconcile after a release:

1. Refresh `schemas/*.graphql` from the subgraphs' `_service { sdl }`.
2. Run `developer/examples/home-graphql/scripts/compose.sh` and commit `supergraph.graphql` and
   `deploy/home-graphql/files/supergraph.graphql`.
3. Only then Sync the ArgoCD Application; the rendered object now matches the
   published content (the three release annotations are the only extra
   metadata).

Until step 2 lands, **do not Sync** `home-graphql` — it would roll the live
schema back.

## Failure behaviour

- Invalid subgraph SDL, a broken `@key`/`@requires` reference or an
  unreachable `schema.subgraph_url` makes Rover exit non-zero. The Task deletes
  any stale artifact, composes into a temp file and moves it into `source` only
  on success, so no artifact exists.
- `publish-configmap` has `runAfter: [compose-supergraph]` and refuses an empty
  artifact, so the ConfigMap is never updated on failure. Hive keeps serving
  the last good supergraph (it keeps the previous in-memory schema when a poll
  yields an unparsable file).
- The ConfigMap Task also compares the content it read back from the API with
  the composed file; a mismatch fails the Task instead of failing silently.

## Scope notes

- **Manual in this iteration, no breaking-change gate.** Nothing diffs the
  composed supergraph against the previous one, and no PaC trigger is wired.
  A field removed from a subgraph disappears from the gateway on the next poll.
- The first compose run downloads the pinned ELv2 plugin from GitHub; later
  runs reuse `.rover-home/.rover` on the PVC and work offline. Subgraph SDLs
  are read from the seeded checkout, not fetched over the network.
- The gateway chart and a later `helm upgrade` still ship the *committed*
  `supergraph.graphql`; see "GitOps deviation" for the commit-before-Sync rule.
- The ConfigMap must stay mounted as a directory (never `subPath`) in the
  gateway Deployment — the kubelet's atomic symlink swap is what the polling
  function opens afresh.

## Values

| Key | Default | Notes |
|---|---|---|
| `configMap.name` | `home-graphql-supergraph` | Default for `config-map-name`; also the Role's `resourceNames` |
| `configMap.namespace` | `home-graphql` | Namespace of the SA/Role/RoleBinding and the source PVC; default for `config-map-namespace` |
| `configMap.key` | `supergraph.graphql` | Single data key; default for the Task's `KEY` |
| `serviceAccount.name` | `supergraph-release` | PipelineRun identity |
| `images.rover` | `ghcr.io/apollographql/rover:1.0.1` | Default for the compose Task's `ROVER_IMAGE` |
| `images.kubectl` | `docker.io/alpine/k8s:1.36.0` | bash + kubectl + tar/base64 (the `registry.k8s.io/kubectl` image is distroless) |
| `rover.plugin` | `supergraph@=2.9.3` | Must match `federation_version` in `developer/examples/home-graphql/supergraph.yaml` |
| `source.pvc.create` | `true` | Render `supergraph-release-source` in `configMap.namespace` |
| `source.pvc.name` | `supergraph-release-source` | Must match `claimName` in the PipelineRun example |
| `source.pvc.size` / `storageClassName` | `1Gi` / `longhorn` | Checkout + artifact + plugin cache |
| `source.seed.configMapName` | `supergraph-source` | Default for `source-configmap`; must be in the PipelineRun namespace |
| `source.seed.key` | `source.tgz.b64` | Data key of the base64 gzip tar |

Every Task/Pipeline default is rendered from these values, so an override moves
the RBAC, the Tasks and the Pipeline together — no forked hardcoded name.

## Rendering

```sh
docker run --rm -v "$PWD:/src" -w /src alpine/helm:latest lint ./charts/supergraph-release
docker run --rm -v "$PWD:/src" -w /src alpine/helm:latest template supergraph-release ./charts/supergraph-release
./tests/supergraph-release-checks.sh
```
