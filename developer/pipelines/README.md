# CI/CD Shared Tekton pipelines

Reusable Tekton tasks and pipelines for tenant CI/CD. Installed once into `tekton-pipelines`; tenant namespaces reference them through the cluster resolver. See the [wiki](../docs/cicd/pipelines/setup/README.md) for details on how to run pipelines in your GitHub repos.

## Prerequisites

- Connect to the home cluster via Pinniped (see `home-cluster-internal/kubernetes/sso-auth-rbac`):

```sh
pinniped login oidc --upstream-identity-provider-name=cognito --request-audience=<kubectl-host>
```

## Pod Security and the task contract

Every Task in this catalog runs under Pod Security `restricted` in a tenant
namespace — verified end-to-end on 2026-10-24 (see the compatibility matrix
below):

- every step runs as **UID/GID 65532** (`runAsNonRoot: true`), with
  `allowPrivilegeEscalation: false` and `capabilities.drop: ["ALL"]`;
- the rootless Buildah build step runs as **UID/GID 1000** (the image's `build`
  user; Buildah calls `user.Current()`), still non-root, still no capabilities;
- no step uses `privileged`, host namespaces, host paths or added capabilities;
- steps that need a writable home set `HOME`/`TMPDIR` explicitly (`/tmp`, or
  `/home/build` for Buildah);
- every step uses `seccompProfile: RuntimeDefault` except the Buildah build
  step, which uses the Localhost profile `profiles/buildah-userns.json`.

### Platform prerequisites (all live on home-cluster-1)

1. **Tekton `set-security-context: "true"`** in the `TektonConfig` pipeline
   section. Tekton injects `prepare`, `place-scripts` and
   `working-dir-initializer` into every TaskRun pod; without this flag those
   containers have no securityContext and PSA rejects the pod before a step
   starts. Keep it in
   `platform/cicd/cicd-tekton-operator/`.
2. **Unprivileged user namespaces** — `user.max_user_namespaces=15000` on every
   node (Talos `machine.sysctls`; repo patch
   `platform/node/patches/user-namespaces.yaml`). Without it rootless
   Buildah/BuildKit fail with `no space left on device` / "user namespaces are
   not enabled".
3. **The `buildah-userns` Localhost seccomp profile** on every node at
   `/var/lib/kubelet/seccomp/profiles/buildah-userns.json`. The stock
   `RuntimeDefault` profile blocks `unshare(CLONE_NEWUSER)` with `EPERM`; the
   profile is the default allowlist plus the namespace, mount and `chroot`
   syscalls a rootless build needs. Source, checksum and Talos install patch:
   `platform/node/patches/buildah-seccomp.yaml`. It is **not** `Unconfined` and only
   build pods use it.
4. **`fsGroup: 65532`** (plus `fsGroupChangePolicy: OnRootMismatch`) on every
   PipelineRun that binds a PVC workspace, so git-clone, npm-install and the
   build/terragrunt steps can all write to the same volume. The usage examples
   under `charts/*/usage-examples/` already do this.

### Security trade-off

The Localhost seccomp profile widens the kernel allowlist for **build pods
only**: it permits creating user/mount namespaces and the mount/`chroot` calls
rootless Buildah uses. The container still has no capabilities, cannot become
root on the host, cannot use host namespaces or host paths, and every other
syscall stays default-deny. The alternative is kaniko, which needs real UID 0
and therefore cannot run in a `restricted` tenant namespace. The profile is a
node file, so it is part of the platform, not of a tenant namespace.

### Restricted compatibility matrix

| Task / object | Runs under `restricted` | Notes |
|---|---|---|
| `git-clone-v1` | yes | UID 65532 |
| `npm-install-v1` | yes | UID 65532, `HOME=/tmp` |
| `gitops-yaml-bump-v1` | yes | UID 65532, local git daemon + GitHub |
| `terragrunt-plan-v1` / `-apply-destroy-v1` | yes | UID 65532, two PVCs OK (`coschedule: disabled`) |
| `build-push-v1` | yes | rootless Buildah, UID 1000, Localhost seccomp |
| `kaniko-v1` | yes | deprecated **alias** of `build-push-v1` (same spec) |
| `app-release-v1` | yes | clone → build-push-v1 → optional bump |
| `supergraph-release-v1` | yes | optional clone → `rover supergraph compose` → ConfigMap apply; UID 65532, writes into the `source` PVC |
| `home-terraform-plans-cleanup` CronJob | yes | UID 65532 |

### One-time migration of existing PVCs

Before this change the tasks ran as root, so existing volumes are root-owned.
`fsGroup` fixes the volume root, but not subdirectories created by old runs:

- `home-terraform-plans` — old `<repo>/<branch>/` directories are root-owned;
  the plan task fails with a clear "not writable by UID 65532" error. Chown the
  volume once (`chown -R 65532:65532 /plans`) or recreate it.
- Workspace PVCs — a PVC that held a root-built repo needs
  `chown -R 65532:65532` once; fresh `volumeClaimTemplate` PVCs are fine.
- The cleanup CronJob cannot delete root-owned leftovers; remove those once by
  hand.

Tenant namespaces are owned by `tenant-platform` (the registry in
`home-cluster-kubernetes-cicd/tenant-platform/values/tenants.yaml`). Tasks and
Pipelines stay in `tekton-pipelines`; PipelineRuns run in the tenant namespace
named by the Repository CR. The shared catalog renders **no** tenant objects:
no `tekton-bot` ServiceAccount and no `npm-npmrc` ConfigMap. `tenant-platform`
owns both per tenant.

## Tenant namespaces and deploy secrets

The shared catalog serves two tiers (spec:
`developer/docs/cicd/pipelines/tenancy/spec.md`):

| | `build` tenant (`stijnklomp-lave`) | `deploy` tenant (operator org) |
|---|---|---|
| Run namespace | `stijnklomp-lave` | `stijnklomp-talos-linux-home-setup` |
| Tasks/Pipelines | `tekton-pipelines` (cluster resolver) | `tekton-pipelines` (cluster resolver) |
| Deploy secrets | **none** | `dockerhub-docker-config`, optional `gitops-pat`/`cloud-creds`/`npm-secret` |
| Plan storage | none | `home-terraform-plans` PVC + cleanup CronJob |
| Kubernetes API egress | denied | allowed (deploy tier CNP) |

The namespace is the secret boundary: a Secret in a tenant namespace is
readable by every PipelineRun in that namespace (secret mounts bypass RBAC), so
operator credentials go only into the deploy tenant namespace.

The `*-secrets` charts and the terragrunt plan PVC/CronJob read a single
namespace from `values/platform.yaml`:

```yaml
namespace: stijnklomp-talos-linux-home-setup
```

That file carries only the deploy tenant namespace. The tenant list itself is
the registry in `home-cluster-kubernetes-cicd/tenant-platform/values/tenants.yaml`
— there is no `namespaces:` list and no three-way sync.

### Out-of-band secrets

Two deploy secrets have committed examples but **do not exist live yet**. The
operator creates them in the deploy tenant namespace after the tenant is
applied; never commit the real files:

```sh
# GitOps PAT used by gitops-yaml-bump-v1 (contents:write on home-cluster)
kubectl -n stijnklomp-talos-linux-home-setup create secret generic gitops-pat \
  --from-literal=username=tekton-bot \
  --from-literal=password='<GITHUB_PAT>'

# Reserved cloud credentials (e.g. Terraform state backend)
kubectl -n stijnklomp-talos-linux-home-setup create secret generic cloud-creds \
  --from-literal=CLOUD_ACCESS_KEY_ID='<ACCESS_KEY_ID>' \
  --from-literal=CLOUD_SECRET_ACCESS_KEY='<SECRET_ACCESS_KEY>'
```

`dockerhub-docker-config` is applied by the `docker-build-publish-secrets`
chart from the gitignored `dockerhub-secret.yaml`; `npm-secret` is optional
(public npm packages need no token).

> **Real (gitignored) secret files must use `.Values.namespace`.** The committed
> `dockerhub-secret.example.yaml` does. An older copy that still loops over
> `.Values.namespaces` renders into `tekton-pipelines` (or nothing after the
> `namespaces` key is gone), so update the gitignored file before applying from
> the main checkout. `tekton-pipelines` must hold no deploy secret.

`kaniko-v1` is kept as a **deprecated alias** with exactly the same rootless
Buildah spec, so existing `.tekton/` callers keep working without a root task in
the catalog. New pipelines should reference `build-push-v1`. It is still
referenced by existing app pipelines, so removing it would break them; remove it
only after every `.tekton/` file uses `build-push-v1`.

## Supergraph release (home-graphql)

`charts/supergraph-release/` is the schema-change path for the Hive Gateway:
compose the Apollo supergraph with the pinned Rover plugin, then publish it as
the gateway's supergraph ConfigMap. Hive polls that ConfigMap (10 s) through a
function source, so a schema change is live without rebuilding the gateway
image or restarting its pod.

| Object | Namespace |
|---|---|
| `Task/supergraph-compose-v1`, `Task/supergraph-configmap-v1`, `Pipeline/supergraph-release-v1` | `tekton-pipelines` |
| `ServiceAccount`/`Role`/`RoleBinding` `supergraph-release` | `home-graphql` |

Trigger it by hand (no PaC trigger, no breaking-change gate in this iteration):

```sh
kubectl apply -f charts/supergraph-release/usage-examples/v1/pipelinerun.yaml
```

A failed composition (invalid SDL, broken `@key`) fails the compose Task and
never reaches the ConfigMap, so Hive keeps the last good schema. Details and
the live-verification procedure: `charts/supergraph-release/README.md` and
`developer/docs/operations/istio-ambient-cutover.md`.

## Notes

- All secrets need to be manually added for Pipelines/Tasks you wish to deploy. The secret files are not source controlled and all have examples which can be copied over into the correct file(s). Most secrets are put in separate releases. *Note that some secret values are only accessible by the owner.*
- App-facing deploy PipelineRuns (copy into your repo's `.tekton/`): `charts/app-release/usage-examples/v1/` — PR build, dev on merge to `main`, staging on a `v*` tag; see `developer/docs/cicd/pipelines/setup/README.md`.
- `npm-install-v1` requires the `npm-npmrc` ConfigMap as its `npmrc` workspace. `tenant-platform` renders the per-tenant copy; this catalog does not render a control-plane copy.
- [Cluster resolvers](https://tekton.dev/docs/pipelines/cluster-resolver/) are limited to the `tekton-pipelines` namespace for security reasons.
- PipelineRuns run in the tenant namespace (the Repository CR namespace is the run namespace); Tasks/Pipelines stay in `tekton-pipelines`. The PaC `pipelinesascode.tekton.dev/target-namespace` annotation is the legacy override — the Repository CR must exist in that namespace too, and the registry is now the source of truth.

## Setup

```sh
helmfile apply --values values/platform.yaml
```

Offline checks (render every chart, assert the catalog owns no tenant objects,
and assert the supergraph-release contract):

```sh
./tests/retire-single-namespace-checks.sh
./tests/supergraph-release-checks.sh
```

Or per stage:

```sh
# git-clone
helmfile -l stage=git-clone apply
```

```sh
# npm-install (public registry via the Verdaccio cache; a token is only needed once a private registry is added)
helmfile -l stage=npm-install apply --values values/platform.yaml

helmfile -l stage=npm-install-secrets apply --values values/platform.yaml
```

```sh
# docker-build-publish: build-push-v1 (rootless Buildah; kaniko-v1 is a
# deprecated alias with the same spec). Push target is set via the IMAGE param
# — Docker Hub now, see the dockerhub-docker-config secret.
helmfile -l stage=docker-build-publish apply

helmfile -l stage=docker-build-publish-secrets apply --values values/platform.yaml
```

```sh
# gitops-yaml-bump (CI -> CD bridge: sets flat env keys in the home-cluster app
# declaration -> ArgoCD ApplicationSet syncs the environment)
helmfile -l stage=gitops-yaml-bump apply

helmfile -l stage=gitops-yaml-bump-secrets apply --values values/platform.yaml
```

```sh
# app-release (Pipeline used by the app-repo PipelineRun examples:
# clone -> build-push-v1 (rootless Buildah) -> optional gitops-yaml-bump)
helmfile -l stage=app-release apply
```

```sh
# supergraph-release (home-graphql): compose the Apollo supergraph with the
# pinned Rover plugin and publish the Hive Gateway ConfigMap. Also renders the
# `supergraph-release` ServiceAccount/Role/RoleBinding into `home-graphql`.
helmfile -l stage=supergraph-release apply
```

```sh
# terragrunt (uses the per-deploy-tenant home-terraform-plans PVC for plan
# handoff; plans are removed after a successful apply, never-applied plans
# swept nightly in the same namespace)
helmfile -l stage=terragrunt apply

helmfile -l stage=terragrunt-secrets apply --values values/platform.yaml
```

### Cleanup

```sh
helmfile destroy
```

## Tips

### Namespaced resource templates

Deploy-tier resources (secrets, plan PVC, cleanup CronJob) render into the one
deploy tenant namespace:

```yaml
namespace: {{ required "namespace is required (apply with --values values/platform.yaml)" .Values.namespace }}
```

Per-tenant objects are **not** templated here. The registry in
`home-cluster-kubernetes-cicd/tenant-platform/values/tenants.yaml` is the only
list of tenants, and `tenant-platform` renders each namespace's quota,
LimitRange, Cilium policy, `tekton-bot` ServiceAccount, `npm-npmrc` and
Repository CRs. A chart in this catalog must not loop over namespaces or create
a ServiceAccount/ConfigMap that belongs to a tenant.

Usage:

```sh
helmfile -l stage=<RELEASE_LABEL> --values values/platform.yaml apply
```