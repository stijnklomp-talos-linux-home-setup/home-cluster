<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-10-24 -->

# AGENTS.md (tenant-platform)

PaC tenancy platform: one registry entry (`values/tenants.yaml`) renders one
tenant namespace plus quota, LimitRange, Cilium policy, `tekton-bot` SA,
`npm-npmrc`, Repository CRs and developer RBAC. A cluster-wide
`ValidatingAdmissionPolicy` binds every Repository CR to its namespace's
`home.cluster/tenant` label (the `pipelines-as-code` namespace is exempt; no global CR exists today). Spec:
`developer/docs/cicd/pipelines/tenancy/spec.md`; ADRs under
`developer/docs/architecture-decision-log/home/`.

## Setup (deploy & teardown)

| Task | Command |
|------|---------|
| Apply | `helmfile -l stage=tenancy apply` (this dir; run from a container per workspace AGENTS) |
| Offline checks | `./tests/render-checks.sh` |
| Live admission checks | `KUBECONFIG_DIR=<cluster-config-dir> ./tests/admission-checks.sh` (creates/deletes `admission-*` CRs only) |
| Live deploy-tenant checks | `KUBECONFIG_DIR=<cluster-config-dir> ./tests/deploy-tenant-checks.sh` (placements, secret isolation, auth can-i, live PipelineRun) |
| Control-plane checks (ticket #8) | `KUBECONFIG_DIR=<cluster-config-dir> ./tests/retire-single-namespace-checks.sh` |
| Clean-rebuild proof (ticket #8) | `KUBECONFIG_DIR=<cluster-config-dir> ./tests/clean-rebuild-checks.sh` (deletes/recreates the build tenant only) |
| Acceptance | see `tests/build-tenant-pipelinerun.yaml`, `tests/deploy-tenant-pipelinerun.yaml` + README |
| Remove a tenant | edit `values/tenants.yaml`, apply; pruner deletes the namespace (PVC data loss — read README) |

Prerequisites: Cilium CRDs, `tekton-pipelines` exists, Tekton
`set-security-context: true`, shared tasks pass `restricted` (ticket #3), and
the `tekton-developer`/`tekton-namespace-lister` ClusterRoles from
`home-cluster-sso-auth-rbac` (the bindings render either way but grant nothing
until those roles exist).

## Commands

```sh
# offline (Docker-first)
./tenant-platform/tests/render-checks.sh

# single render
docker run --rm -v "$PWD/tenant-platform:/src" -w /src alpine/helm:latest \
  template tenant-platform ./charts/tenant-platform -f values/tenants.yaml

# helmfile render
docker run --rm -v "$PWD/tenant-platform:/src" -w /src \
  ghcr.io/helmfile/helmfile:latest template
```

## Key files

| Path | Notes |
|------|-------|
| `values/tenants.yaml` | **THE REGISTRY** — source of truth; no other list of namespaces/repos |
| `charts/tenant-platform/templates/_helpers.tpl` | registry validation (`fail`), developer slug, deduped developer list |
| `charts/tenant-platform/templates/validate.yaml` | calls the validator; renders nothing |
| `charts/tenant-platform/templates/namespaces.yaml` | Namespace + `home.cluster/tenant`, `home.cluster/tier`, `home.cluster/managed`, restricted PSA |
| `charts/tenant-platform/templates/quotas.yaml` | ResourceQuota + LimitRange per tier with per-tenant overrides |
| `charts/tenant-platform/templates/networkpolicy.yaml` | `tenant-sandbox` CiliumNetworkPolicy; API allow only for `deploy` |
| `charts/tenant-platform/templates/repositories.yaml` | one Repository CR per `repos[]`, `source` provenance |
| `charts/tenant-platform/templates/admission-policy.yaml` | cluster-wide `ValidatingAdmissionPolicy` + binding: Repository URL owner must match the namespace label; `pipelines-as-code` exempt |
| `charts/tenant-platform/templates/developer-rbac.yaml` | per-tenant RoleBindings + deduped namespace-lister CRBs |
| `charts/tenant-platform/templates/prune-hook.yaml` | post-install/post-upgrade Job + least-privilege RBAC; refuses an empty registry unless `pruner.allowEmptyRegistry=true` |
| `charts/tenant-platform/values.yaml` | tier quota/LimitRange defaults, Verdaccio endpoint, npmrc body, pruner image |
| `tests/render-checks.sh` | offline render/validation + prune-logic checks |
| `tests/admission-checks.sh` | live admit/deny checks for the Repository gate (creates/deletes `admission-*` CRs only) |
| `tests/build-tenant-pipelinerun.yaml` | live build-tenant acceptance PipelineRun (cluster resolver + inline probe) |
| `tests/deploy-tenant-pipelinerun.yaml` | live deploy-tenant acceptance PipelineRun (inline probe: API + DNS + Verdaccio + internet) |
| `tests/deploy-tenant-checks.sh` | live deploy-tenant checks: CR counts/placement, deploy secrets + PVC/CronJob, build-tenant isolation, PipelineRun |
| `tests/retire-single-namespace-checks.sh` | live control-plane emptiness: no tenant objects/releases in `tekton-pipelines`; tenant objects only in tenant namespaces |
| `tests/clean-rebuild-checks.sh` | live clean rebuild: deletes the build namespace, `helmfile sync` from Git, asserts every tenant object and re-runs acceptance |

## Conventions & rules

- **Never derive a namespace from an owner at runtime.** The registry carries
  the explicit alias; uppercase logins are valid owners but invalid namespaces.
- The chart owns only platform objects. PaC owns PipelineRuns/TaskRuns/pods/PVCs
  and the per-run git auth secret; the SSO/RBAC repo owns the `tekton-developer`
  and `tekton-namespace-lister` ClusterRoles.
- A tenant namespace must never receive operator credentials (Docker Hub, GHCR,
  AWS, GitOps) or deploy credentials for a `build` tenant.
- Do not add a Secret template to this chart. The deploy tenant's secrets,
  plan PVC and cleanup CronJob are rendered by `developer/pipelines`
  with `values/platform.yaml` (`namespace: stijnklomp-talos-linux-home-setup`);
  this chart owns the namespace they land in.
- `gitops-pat` and `cloud-creds` have examples only and do not exist live yet;
  the operator creates them out of band in the deploy tenant namespace. Never
  commit the real files.
- `repos[].name` is the Repository CR name **and** must be unique per namespace.
- The admission gate (`tenant-platform-repository-owner`) is defense-in-depth,
  not the primary control. It matches `pipelinesascode.tekton.dev/v1alpha1`
  `repositories` CREATE/UPDATE cluster-wide with `failurePolicy: Fail` and
  `validationActions: [Deny]`; the `pipelines-as-code` namespace is exempt so
  a global Repository CR (if one is added) still works. Do not remove or weaken it without a
  replacement, and do not add a tenant namespace to the exemption.
- Owner derivation is fixed to the 4th `/`-segment of `spec.url`. GitHub logins
  are ASCII, so the comparison uses `lowerAscii()`; labels keep the owner
  verbatim (`stijnklomp-LAVE`). URLs shorter than four segments are denied.
- Old single-namespace Repository CRs in `tekton-pipelines` cannot be created or
  updated after this gate is live (that namespace has no tenant label). They
  were removed in ticket #8; the gate keeps the retired path from returning.
  Status subresource writes are not matched.
- PaC auto-configure is off in the `pipelines-as-code` ConfigMap; the registry is
  the only way a repository becomes PaC-enabled. When debugging "no run", check
  the registry first and the controller log for
  `AutoConfigureNewGitHubRepo: from 'true' to 'false'` after the change.
- The pruner's `home.cluster/managed=true` label and the
  `tekton-namespace-lister-` CRB prefix are reserved. Pruner RBAC must stay
  `namespaces list/delete` + `clusterrolebindings list/delete` — no `get`, no
  wildcards.
- Namespace deletion is destructive: Longhorn PVC data is removed with the
  namespace. Never prune a namespace with data you cannot lose.
- `source` provenance is intentional: tenants may change `.tekton/` in a feature
  branch; the sandbox (PSA + quota + Cilium + namespace-scoped secrets) is the
  safety boundary.
- Cilium must be the CNI; on Flannel the CiliumNetworkPolicy is accepted and
  ignored.
- Cross-repo rules: workspace root `AGENTS.md`.

## Security

- No real secrets, keys, personal data or LAN IPs committed. The acceptance
  manifest uses the already-public cluster endpoint (`192.168.1.107:6443`).
- No operator credentials in tenant namespaces. Secret mounts bypass RBAC.
- The prune hook runs with cluster-scoped delete on namespaces; keep its RBAC
  minimal and review any change.

## Checklist

- [ ] `./tests/render-checks.sh` passes (includes duplicate/format validation)
- [ ] live `tests/admission-checks.sh` passes after changing the policy
- [ ] live `tests/retire-single-namespace-checks.sh` passes (no tenant objects in `tekton-pipelines`)
- [ ] registry namespaces are explicit DNS-1123 labels
- [ ] `helmfile template` renders
- [ ] no Secret added to the chart
- [ ] no tenant name hardcoded in templates
- [ ] developer RBAC is deduped across tenants
- [ ] README pruning warning still accurate

## Examples

The real registry (`values/tenants.yaml`) has two tenants:

- build: `stijnklomp-LAVE` → `stijnklomp-lave`, one repo (`test-app`), one
  developer binding (`stijnklomp1@hotmail.com`) used by the live RBAC
  acceptance check in the SSO/RBAC repo;
- deploy: `stijnklomp-talos-linux-home-setup` →
  `stijnklomp-talos-linux-home-setup`, all 17 org repositories, the same
  developer, deploy secrets and the terragrunt plan PVC.

`tests/fixtures/valid-two-tenants.yaml` is a build+deploy fixture used by the
offline checks.

## When stuck

- Spec: `developer/docs/cicd/pipelines/tenancy/spec.md`; ADRs under
  `developer/docs/architecture-decision-log/home/`.
- Network: `kubectl get cnp -n <ns>`, Cilium/Hubble, PSA events.
- Admission gate: `kubectl get validatingadmissionpolicy tenant-platform-repository-owner`,
  then `tests/admission-checks.sh`; the denial message names the
  `home.cluster/tenant` label.
- The old single-namespace PaC repositories chart was deleted in ticket #8 and
  its Helm release was uninstalled. `tekton-pipelines` holds shared
  Tasks/Pipelines and Tekton configuration only; do not recreate the chart or
  render a tenant SA/`npm-npmrc`/Repository CR into the control plane.

## Related docs

Operator runbook (add/remove/offboard a tenant, secrets, RBAC, troubleshooting, rollback):
[`developer/docs/cicd/pipelines/tenancy/runbook.md`](https://github.com/stijnklomp-talos-linux-home-setup/home-cluster/blob/main/developer/docs/cicd/pipelines/tenancy/runbook.md).
