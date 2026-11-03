<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-03 -->

# AGENTS.md (platform/cicd)

CI/CD layer of the home cluster: Tekton operator + Pipelines-as-Code + NPM cache registry + ArgoCD (GitOps CD). Each subdirectory is its own helmfile project with its own AGENTS.md.

## Overview
Overview of this repo — see `README.md` for prose.

## Setup (deploy & teardown)

Deploy in dependency order, `helmfile apply` inside each component dir:

| Task | Command |
|------|---------|
| Tekton operator | `cd cicd-tekton-operator && helmfile -l stage=operator apply` then `helmfile -l stage=config apply` (must be first — PaC needs Tekton Pipelines) |
| Pipelines-as-Code | `cd cicd-tekton-pipelines-as-code && helmfile -l stage=expose-operator apply` → `gosmee` (GitHub App secret first: `kubectl apply -f secrets/pipelines-as-code-github-secrets.yaml`); Repository CRs come from `tenant-platform` |
| Verdaccio | `cd cicd-verdaccio && helmfile apply` |
| Tenant platform | `cd tenant-platform && helmfile -l stage=tenancy apply` (needs Cilium CRDs + Tekton operator; registry = `values/tenants.yaml`) |
| ArgoCD | `cd cicd-argocd && helmfile -l stage=argocd apply` (needs `secrets/argocd-secrets.yaml` + `secrets/argocd-github-creds.yaml` first — see example) |
| Cleanup | `helmfile destroy` per dir, reverse order |

## Commands
Full command table lives in the Setup section; offline validation: `helm template <rel> ./charts/<name>` and `helm lint` via the alpine/helm container (see workspace root AGENTS.md).

## Key files

| Path | Notes |
|------|-------|
| `cicd-tekton-operator/` | Tekton Operator 0.81.1 + TektonConfig + standalone write-mode dashboard (per-user impersonation) |
| `cicd-tekton-pipelines-as-code/` | PaC v0.50.0 + GitHub App + gosmee webhook relay (no domain needed); auto-configure off |
| `tenant-platform/values/tenants.yaml` | **THE REGISTRY** — the only list of tenants, namespaces and Repository CRs (no separate namespace list or sync) |
| `cicd-verdaccio/` | cache-only NPM registry (ClusterIP, no auth, Longhorn PVC) |
| `tenant-platform/` | PaC tenancy registry + chart: tenant namespaces, quotas, Cilium policy, Repository CRs, admission gate, prune hook; offline checks in `tests/render-checks.sh`, live gate checks in `tests/admission-checks.sh` |

## Conventions & rules

- No root `helmfile.yaml` — each component dir is a standalone helmfile project (it is also its own git-tracked unit).
- Deploy order matters: `cicd-tekton-operator` first; `cicd-tekton-pipelines-as-code` and `cicd-verdaccio` both need the base layer (Istio edge Gateway, Longhorn) from `platform/cluster`.
- Component-specific rules live in each subdir's AGENTS.md — read it before editing that component.
- Never edit vendored `charts/upstream/*.tgz` or `kustomizations/upstream/*.yaml` in place; refresh via documented `helm pull` / curl.
- Secrets live per component under `secrets/`; real files are gitignored — only `*.example.yaml` is committed.
- Cross-repo rules: workspace root `AGENTS.md`.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- `<ingress-ip>`, `<your-github-org>`, `<region>`, `<user-pool-id>` are TODOs, not values to invent.

## Checklist
- [ ] helmfile chart paths / terragrunt sources resolve (no dangling `./charts/...`)
- [ ] YAML parses (`helm template` or a yaml lint)
- [ ] Repository CRs are rendered by `tenant-platform`, never by the PaC component
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value

## Examples
`cicd-tekton-operator` is the reference helmfile pattern; `tenant-platform` shows the registry-driven approach.

## When stuck
- For pipeline/auth issues: `developer/docs/cicd/pipelines/troubleshoot/` and the restricted wiki runbooks.
