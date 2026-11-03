<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-10-24 -->

# AGENTS.md (cicd-tekton-operator)

Tekton Operator (0.81.1) + the cluster's TektonConfig + standalone Tekton Dashboard (v0.72.0, write-mode, per-user impersonation). Log retention is TODO (later date).

## Overview
Overview of this component — see `README.md` for prose.

## Setup (deploy & teardown)

| Task | Command |
|------|---------|
| Operator | `helmfile -l stage=operator apply` |
| Config | `helmfile -l stage=config apply` |
| Cleanup | `helmfile -l stage=config destroy` then `helmfile -l stage=operator destroy` |
| Node roles | `platform/node/designate-node-roles.yaml` (CI node label/taint + worker labels, before TaskRuns run) |

## Commands
Full command table lives in the Setup section; offline validation: `helm template <rel> ./charts/<name>` and `helm lint` via the alpine/helm container (see workspace root AGENTS.md).

## Key files

| File | Notes |
|------|-------|
| `charts/upstream/tekton-operator-0.81.1.tgz` | vendored from GitHub release (not a Helm repo) |
| `kustomizations/tekton-operator-config/resources/config.yaml` | TektonConfig: profile basic, pruner (hourly, 24h), cluster-resolver allow-list `tekton-pipelines` (shared Tasks), CI-node default pod template with `fsGroup: 65532`/`fsGroupChangePolicy: OnRootMismatch`, `set-security-context: true` (restricted tenant namespaces), `coschedule: disabled` (multi-PVC terragrunt pipelines), operators/webhooks on the worker pool, `result.disabled` |
| `kustomizations/upstream/tekton-dashboard-0.72.0.yaml` | upstream **write-mode** (`release-full.yaml`) dashboard manifest, vendored (refresh via documented curl in README) |
| `kustomizations/tekton-dashboard/` | dashboard overlay: **strips ALL SA RBAC** (7 ClusterRoleBindings + 3 ClusterRoles — per-user impersonation instead), worker-pool nodeSelector, **no `--namespaces` pin** (`--namespaces=`; impersonation + RBAC decides which namespaces/resources are visible), `--logout-url` to oauth2-proxy |

## Conventions & rules

- cluster-resolver `allowed-namespaces` is `tekton-pipelines` only. Tenant PipelineRuns run in their own namespaces but resolve shared Tasks from `tekton-pipelines`; the allow-list constrains the resolved object's namespace, not the run namespace.
- `set-security-context: true` is required for tenant namespaces: Tekton-injected init/sidecar containers otherwise have no securityContext and `restricted` PSA rejects every tenant TaskRun. It only hardens Tekton-injected containers; task steps must set their own (ticket #3). Do not remove it before Tekton Pipelines v1.16 (where it defaults to true).
- `coschedule: disabled` replaces the deprecated `disable-affinity-assistant`: with the operator default (`workspaces`) Tekton rejects a TaskRun that binds two PVCs (`more than one PersistentVolumeClaim is bound`), and the terragrunt plan/apply pipelines bind `source` + `plans`.
- Dashboard is deliberately **NOT operator-managed** (`profile` stays `basic`): the operator would restore the cluster-wide ClusterRoleBindings the overlay strips. Keep it standalone.
- Dashboard RBAC = per-user via oauth2-proxy impersonation (`home-sso-auth-rbac/charts/expose-tekton-dashboard` + `oauth-proxy-impersonator-binding-tekton`). Never re-add SA bindings to the dashboard.
- The dashboard has **no namespace pin**. Visibility is the impersonated user's RBAC: admins get the cluster-wide `admin-read`/`admin-write`; tenant developers get `tekton-developer` bound per tenant namespace by `tenant-platform`, plus `tekton-namespace-lister` for the namespace dropdown. Do not add `--namespaces` back — a pin would hide tenant namespaces from their developers.
- A tenant developer binding is read-only by design: `create` in a namespace is equivalent to that namespace's workload identity. `tekton-developer` must never gain secrets, configmaps, write verbs or `pods/exec|attach|portforward`.
- Do not re-add vector/minio (log retention) without updating this AGENTS.md + the ADRs.
- Version bump: `tekton-operator-x.y.z.tgz` from https://github.com/tektoncd/operator/releases — update README + helmfile together. Dashboard bump: re-vendor `release-full.yaml` from https://infra.tekton.dev/tekton-releases/dashboard/previous/ and re-check the args patch (full-list replace) + RBAC strip.
- Cross-repo rules: workspace root `AGENTS.md`.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- `<ingress-ip>`, `<your-github-org>`, `<region>`, `<user-pool-id>` are TODOs, not values to invent.

## Checklist
- [ ] helmfile chart paths / terragrunt sources resolve (no dangling `./charts/...`)
- [ ] YAML parses (`helm template` or a yaml lint)
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value

## Examples
`cicd-tekton-operator` is the reference helmfile pattern; `tenant-platform` shows the registry-driven approach.

## When stuck
- For pipeline/auth issues: `developer/docs/cicd/pipelines/troubleshoot/` and the restricted wiki runbooks.
