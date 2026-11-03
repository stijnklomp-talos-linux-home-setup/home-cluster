<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-03 -->

# AGENTS.md (developer/pipelines)

Reusable Tekton tasks/pipelines installed once into the cluster and referenced via cluster-resolver from app repos.

## Overview
Overview of this component — see `README.md` for prose.

## Setup (deploy & teardown)

| Task | Command |
|------|---------|
| All | `helmfile apply --values values/platform.yaml` (in this dir) |
| Partial | `helmfile -l stage=<git-clone\|npm-install\|docker-build-publish\|gitops-yaml-bump\|app-release\|supergraph-release\|terragrunt> apply` |
| Secrets releases | `helmfile -l stage=<...-secrets> apply --values values/platform.yaml` (deploy tenant namespace) |
| Offline retire checks | `./tests/retire-single-namespace-checks.sh` (no tenant SA/ConfigMap in the catalog) |
| Cleanup | `helmfile destroy` |

## Charts (all in `charts/`; Tasks/Pipelines in `tekton-pipelines`, PipelineRuns and deploy secrets in tenant namespaces)

Per-tenant objects (namespace, quota, LimitRange, Cilium policy, `tekton-bot`
ServiceAccount, `npm-npmrc`) are owned by `tenant-platform`, never by this
catalog.

| Chart | Task/pipeline | Notes |
|-------|---------------|-------|
| `git-clone` | `git-clone-v1` | tektoncd catalog fork |
| `npm-install` | `npm-install-v1` | installs via the Verdaccio cache (`cicd-verdaccio`); reads the `npm-npmrc` ConfigMap as the `npmrc` workspace (owned by `tenant-platform` in every tenant namespace); UID 65532, `HOME=/tmp` |
| `npm-install-secrets` | `npm-secret` | optional private-registry token; leave unset/empty for the public registry |
| `docker-build-publish` | `build-push-v1` (rootless Buildah, canonical) + `kaniko-v1` (deprecated **alias**, identical spec) | runs under `restricted` with the `profiles/buildah-userns.json` Localhost seccomp profile (Talos patch: `platform/node/patches/buildah-seccomp.yaml`); no root task remains. `npm-secret` ref is `optional: true` |
| `docker-build-publish-secrets` | `ghcr-docker-config` + `dockerhub-docker-config` | placeholders; dockerhub entry key must be `https://index.docker.io/v1/` |
| `gitops-yaml-bump` | `gitops-yaml-bump-v1` | **CI→CD bridge**: clones the GitOps repo (home-cluster), sets flat top-level keys (e.g. `imageTagDev=<sha>`, `imageTagStaging=v1.2.3`) in the app declaration, commits + pushes. `[skip ci]` in the commit so PaC does not re-trigger; ArgoCD's ApplicationSet syncs the env. UID 65532, `HOME=/tmp`, `safe.directory=/workspace` |
| `gitops-yaml-bump-secrets` | `gitops-pat` | PAT with `contents:write` on the GitOps repo (used as Bearer token, not basic auth) |
| `app-release` | `app-release-v1` pipeline | clone → build-push-v1 (rootless Buildah) → optional gitops-yaml-bump; app-facing PipelineRun examples in `usage-examples/v1/` (PR build, dev on merge to main, staging+prod record on `v*` tag). Runs under `restricted` |
| `supergraph-release` | `supergraph-seed-v1` + `supergraph-compose-v1` + `supergraph-configmap-v1` + `supergraph-release-v1` pipeline | **manual** schema-change path for Hive Gateway (`home-graphql`): `git-clone-v1` (when `repo-url` is set) **or** `supergraph-seed-v1` (unpacks the `supergraph-source` base64-tar ConfigMap into the `supergraph-release-source` PVC; `home-graphql` has no reachable remote) → `rover supergraph compose` (pinned 1.0.1 + `supergraph@=2.9.3`, `--skip-update`, plugin cached in `.rover-home/`) → render + `kubectl apply` on the existing `home-graphql-supergraph`. Namespaced parts: the `supergraph-release` SA/Role/RoleBinding (**no `create`**, `get/update/patch` with `resourceNames`) and the source PVC, rendered into `home-graphql` (values `configMap.namespace`), never a tenant. Install **after** the gateway chart. Writes the ArgoCD-managed ConfigMap directly: the Application goes OutOfSync until the recomposed artifact is committed; see `charts/supergraph-release/README.md` "GitOps deviation". No PaC trigger, no breaking-change gate. `tests/supergraph-release-checks.sh` |
| `terragrunt` | `terragrunt-plan-v1` + `terragrunt-apply-destroy-v1` | plan→PVC handoff, no cloud; the apply task deletes the plan from the shared PVC after a successful apply (kept on failure for debugging). UID 65532, `HOME=/tmp` |
| `terragrunt-secrets` | `home-terraform-plans` PVC + `home-terraform-plans-cleanup` CronJob + `cloud-creds` placeholder | all in the deploy tenant namespace from `values/platform.yaml` (per-deploy-tenant plan storage); nightly CronJob sweeps never-applied plan files older than 30 days as UID 65532 (same identity as the tasks) |

## Deploy logic for app repos (copy these PipelineRuns)

`charts/app-release/usage-examples/v1/` holds the PipelineRuns developers copy
into an app repo's `.tekton/` directory (or generate with
`platform/gitops/scripts/new-service.sh`):

| Example file | PaC trigger | What it does |
|--------------|-------------|--------------|
| `ci-pull-request-pipelinerun.yaml` | `pull_request` → `main` | build + push `:<sha>`; `gitops-file` is empty, so the GitOps bump is skipped |
| `cd-dev-pipelinerun.yaml` | `push` → `refs/heads/main` | build `:<sha>`, set `imageTagDev` → ArgoCD auto-syncs `<app>-dev` |
| `cd-staging-pipelinerun.yaml` | `push` → `refs/tags/v*` | build `:<git_tag>`, set `imageTagStaging`/`targetRevisionStaging` + `imageTagProd`/`targetRevisionProd` → `<app>-staging` auto-syncs, `<app>-prod` waits for a manual Sync in ArgoCD |

- Replace `docker.io/<DOCKERHUB_USERNAME>/<APP>` and `developer/apps/baremetal/<APP>.yaml` (the app declaration in the `home-cluster` repo).
- `gitops-set` entries must be flat top-level keys that exist in the declaration — the bump task fails if a key is not found (it never silently adds new keys).
- Required PipelineRun workspaces/secrets: `source` (PVC template), `basic-auth` (`{{ git_auth_secret }}`), `docker-credentials` (`dockerhub-docker-config`), `gitops-auth` (`gitops-pat`, contents:write on `home-cluster`).
- The examples set `taskRunTemplate.podTemplate.securityContext.fsGroup: 65532`; keep it on every copied PipelineRun so the UID-65532 steps can write the PVC. Under tenancy the same PipelineRun runs in the tenant namespace under `restricted`.
- The app must be declared once in `home-cluster` (`developer/apps/<cluster>/<app>.yaml`) for the generated `<app>-<env>` Applications to exist — env model: `developer/docs/architecture-decision-log/home/gitops-environments-adr.md`.

## Conventions & rules

- **Pod Security `restricted`**: every step runs as UID/GID 65532 (the build step as UID 1000 — Buildah's `build` user), with `runAsNonRoot`, `allowPrivilegeEscalation: false` and `capabilities.drop: ["ALL"]`. No `privileged`, host namespaces, host paths or added capabilities anywhere; `kaniko-v1` is a deprecated alias of the same rootless task. All steps use `RuntimeDefault` seccomp except the Buildah step, which uses the `profiles/buildah-userns.json` Localhost profile.
- **The build seccomp profile is a node prerequisite**: the stock `RuntimeDefault` profile blocks `unshare(CLONE_NEWUSER)` (EPERM), so rootless builds need `user.max_user_namespaces=15000` on the nodes **and** `/var/lib/kubelet/seccomp/profiles/buildah-userns.json`. Source + Talos patch: `platform/node/patches/buildah-seccomp.yaml` (staged with `--mode=staged`, applied at the next reboot). The profile is the default allowlist plus namespace/mount/chroot syscalls, not `Unconfined`, and only build pods use it.
- **Tekton must set `pipeline.set-security-context: "true"`** in the `TektonConfig` (operator repo) — without it Tekton's injected `prepare`/`place-scripts`/`working-dir-initializer` containers have no securityContext and PSA rejects every restricted TaskRun. It is live on the home cluster; keep it in the operator config.
- Every PipelineRun that binds a PVC workspace sets `podTemplate.securityContext.fsGroup: 65532` (and `fsGroupChangePolicy: OnRootMismatch`) so all steps can write to the shared volume. `source` workspaces are PVCs, not host paths.
- `values/platform.yaml` is the shared namespace input: `namespace` is the **deploy tenant** (operator org) namespace that owns the deploy secrets and the terragrunt plan PVC. There is no `namespaces` list: the only tenant list is the registry in `home-cluster-kubernetes-cicd/tenant-platform/values/tenants.yaml` — there is no three-way namespace sync.
- PipelineRuns run in the tenant namespace (Repository CR namespace = run namespace); Tasks/Pipelines stay in `tekton-pipelines` and the cluster resolver reads only `tekton-pipelines`.
- `npm-install-v1` requires the `npm-npmrc` ConfigMap as its `npmrc` workspace. `tenant-platform` owns the per-tenant copy; this catalog renders no `npm-npmrc` and no `tekton-bot` ServiceAccount.
- `tekton-pipelines` holds shared Tasks/Pipelines and Tekton configuration only. Never render a tenant Repository CR, ServiceAccount, `npm-npmrc` or deploy secret into it from this catalog.
- The one exception to "no ServiceAccount" is `charts/supergraph-release`: its `supergraph-release` SA/Role/RoleBinding and the `supergraph-release-source` PVC render into the **ConfigMap namespace** (`home-graphql`), not into a tenant and not into `tekton-pipelines`. The Role is scoped with `resourceNames` to the single supergraph ConfigMap and grants **no `create`** (that verb cannot be resource-scoped and would allow writing any ConfigMap in the namespace). Consequence: apply the gateway chart first; the publisher fails with "does not exist" rather than creating an unmanaged object.
- **`supergraph-release` writes an ArgoCD-managed ConfigMap.** The `home-graphql` Application owns `home-graphql-supergraph`, so every release leaves it OutOfSync. This is a documented deviation because `home-graphql` has no pushable remote; after a release, recompose locally (`scripts/compose.sh`), commit `supergraph.graphql` + the chart's `files/` copy, then Sync. Never Sync before that commit — it reverts the live schema.
- `home-graphql` has no reachable git remote from the cluster, so the release path seeds the PVC from the `supergraph-source` ConfigMap (`source.tgz.b64`, created by hand from the checkout; unlabeled so neither Helm nor ArgoCD prunes it). The seed Task refreshes the checkout every run and preserves `.rover-home/` (Rover's plugin cache); compose deletes the previous artifact before composing.
- `gitops-yaml-bump-v1` edits **flat top-level** `key: value` lines only — keys are case-sensitive and must exactly match the app declaration in `home-cluster`. It runs as UID 65532 and adds `/workspace` to `safe.directory` because Tekton's shared emptyDir is root-owned.
- Tekton rejects array params (`$(params.X[*])`) inside a `script` body (`variable type invalid`); forward them via `args:` and use `$@`/`$*` in the script (see the bump task's `commit-and-push` step).
- The app-repo PipelineRun examples are the developer contract: when the deploy flow changes, update the examples **and** the `new-service.sh` substitutions together.
- Secrets: commit only `*.example.yaml`; real files are gitignored. A secrets chart with no real file renders nothing and helmfile skips it (no release) — that is expected until the secret is created.
- Real (gitignored) secret files must use `.Values.namespace` like the committed examples. A stale file that loops over `.Values.namespaces` renders into the wrong namespace (or nothing after the `namespaces` key was removed); `tekton-pipelines` must hold no deploy secret.
- Plan/apply uses the `home-terraform-plans` RWO PVC — only one plan+apply pair at a time (TODO in `apply-destroy-task.yaml`). Plans are cleaned up by the apply task (on success) and the `home-terraform-plans-cleanup` CronJob (>30 days, nightly, now also UID 65532). Files left by the old root-based tasks must be removed once by hand.
- Multiple distinct PVC workspaces in one TaskRun require `coschedule: disabled` (the live `TektonConfig` now sets it; the repo's stale `disable-affinity-assistant` key was superseded). Verified 2026-10-24 with the terragrunt `source` + `plans` pair.
- Task pods run on the worker node (TektonConfig default pod template); cluster-resolver only reads `tekton-pipelines`.
- Verify image tags before pinning: `docker.io/alpine/git:2.47.0` did not exist and is now `2.47.2`; the `git-init` and Buildah tags still carry TODOs in the task files.

## Commands
Full command table lives in the Setup section; always pass `--values values/platform.yaml` for namespace-templated charts.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- `<ingress-ip>`, `<your-github-org>`, `<region>`, `<user-pool-id>` are TODOs, not values to invent.

## Checklist
- [ ] helmfile chart paths / terragrunt sources resolve (no dangling `./charts/...`)
- [ ] YAML parses (`helm template` or a yaml lint)
- [ ] Tenant namespace changes agree with the registry (`tenant-platform/values/tenants.yaml`)
- [ ] No chart renders a per-tenant ServiceAccount or `npm-npmrc` (tenant-platform owns them)
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value

## Examples
`charts/app-release/usage-examples/v1/` = the app-facing deploy PipelineRuns (PR / dev / staging); other `charts/*/usage-examples/` show task-level PaC + cluster-resolver wiring. End-to-end flow: `developer/docs/cicd/pipelines/setup/README.md`; declaration/ApplicationSet side: `platform/gitops/AGENTS.md`.

## When stuck
- For pipeline/auth issues: `developer/docs/cicd/pipelines/troubleshoot/` and the restricted wiki runbooks.
