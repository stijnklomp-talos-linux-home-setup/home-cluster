<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-01 -->

# AGENTS.md (home-cert-manager)

Day-0 base component: cert-manager + ClusterIssuers. LAN certs come from the internal CA `home-issuer`; public/Cloudflare DNS-01 is reserved (disabled — no domain yet).

## Overview
Overview of this component — see `README.md` for prose.

## Setup (deploy & teardown)

| Task | Command |
|------|---------|
| Apply | `helmfile apply` (this dir) |
| Cleanup | `helmfile destroy` |

## Commands
Full command table lives in the Setup section; offline validation: `helm template <rel> ./charts/<name>` and `helm lint` via the alpine/helm container (see workspace root AGENTS.md).

## Key files

| File | Notes |
|------|-------|
| `charts/upstream/cert-manager-v1.21.1.tgz` | pinned; refresh via `helm pull jetstack/cert-manager --version <latest>` |
| `kustomizations/home-issuers/root-ca.yaml` | `home-root-ca` self-signed ClusterIssuer (root of trust) |
| `kustomizations/home-issuers/home-issuer.yaml` | intermediate issuer used by every LAN ingress |
| `kustomizations/home-issuers/cloudflare-issuer.yaml` | COMMENTED OUT — enable only when a domain exists |

## Conventions & rules

- `home-issuer` name is referenced by many charts (ingresses, certificates) across the workspace — renaming breaks everything; ask first.
- Root CA (`home-root-ca` Secret) must be distributed to devices' trust stores; instructions in README.
- Rotating the root CA invalidates every issued cert — never rotate casually.
- When enabling Cloudflare DNS-01: uncomment the issuer AND let the workspace know (`home-cloudflare-tunnel`, wiki access guide, PaC ingress).
- Cross-repo rules: workspace root `AGENTS.md`.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- `<ingress-ip>`, `<your-github-org>`, `<region>`, `<user-pool-id>` are TODOs, not values to invent.

## Checklist
- [ ] helmfile chart paths / terragrunt sources resolve (no dangling `./charts/...`)
- [ ] YAML parses (`helm template` or a yaml lint)
- [ ] No tenant/namespace list duplicated outside the PaC tenant registry (`tenant-platform/values/tenants.yaml`)
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value

## Examples
`home-metallb` is the reference helmfile pattern; `home-istio` shows the edge Gateway and the presync namespace hook.

## When stuck
- For pipeline/auth issues: `developer/docs/cicd/pipelines/troubleshoot/` and the restricted wiki runbooks.
