<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-03 -->

# AGENTS.md (developer/docs)

Public documentation for the home cluster: ADRs, access guides, pipeline docs. No code — markdown only. Part of the `home-cluster` monorepo; the restricted counterpart is the separate `home-cluster-internal` repo.

## Overview
Overview of this component — see `README.md` for prose.

## Setup
| Nothing to deploy | Docs-only directory |


## Commands
None — docs only.

## Structure

| Path | Contents |
|------|----------|
| `README.md` | Index |
| `GLOSSARY.md` | Vocabulary: tenancy, CI/CD, service networking |
| `architecture-decision-log/` | `home/` = ADRs for this cluster; `templates/standard-adr-template.md` = format |
| `cicd/access/` | SSO login + Pinniped kubeconfig guide |
| `cicd/pipelines/{setup,tips,troubleshoot,retention}/` | How to run Tekton pipelines via PaC in tenant namespaces |
| `cicd/pipelines/tenancy/spec.md` | PaC multi-tenant contract: registry, run namespaces, tiers, RBAC, admission, network policy |
| `cicd/pipelines/tenancy/runbook.md` | Operator runbook: add/remove a tenant, secrets, developer visibility, network policy, troubleshooting, rollback |
| `operations/` | Cluster operations runbooks: Istio edge/ambient cutover, rollback and deferred follow-ups |
| `tools/jwt/` | Local JWT inspector (Docker) |

## ADR format (MUST follow template)

`Status/Context/Decision/Rationale/Consequences`. New ADR → copy `templates/standard-adr-template.md`.

## Conventions & rules

- **No secrets, no kubeconfigs with real creds, no IPs of the home LAN** — this repo is public.
- Sensitive runbooks/ADRs go in `home-cluster-internal/wiki/` (restricted) and are linked, not duplicated.
- Keep docs succinct; tables over prose.
- When a domain + Cloudflare Tunnel land, update `cicd/access/README.md` (LAN vs WAN URL tables).

## Golden samples

| For | Reference |
|-----|-----------|
| ADR | `architecture-decision-log/home/auth-adr.md` |
| Access guide | `cicd/access/README.md` |

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- `<ingress-ip>`, `<your-github-org>`, `<region>`, `<user-pool-id>` are TODOs, not values to invent.

## Checklist
- [ ] helmfile chart paths / terragrunt sources resolve (no dangling `./charts/...`)
- [ ] YAML parses (`helm template` or a yaml lint)
- [ ] The registry (`tenant-platform/values/tenants.yaml`) is the only tenant/namespace list; no duplicate namespace list or three-way sync
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value

## Examples
`developer/docs/architecture-decision-log/home/auth-adr.md` and the standard-adr-template are the ADR references.

## When stuck
- For pipeline/auth issues: `developer/docs/cicd/pipelines/troubleshoot/` and the restricted wiki runbooks.

## When instructions conflict
Workspace root `AGENTS.md` applies for cross-repo rules (placeholders, secrets, versions).
