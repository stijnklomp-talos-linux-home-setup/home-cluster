<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-01 -->

# AGENTS.md (home-metallb)

Day-0 base component: bare-metal LoadBalancer (L2). Required before the Istio edge Gateway (and every other LoadBalancer Service) can get an IP.

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
| `charts/upstream/metallb-0.16.1.tgz` | pinned; refresh: `helm pull metallb/metallb --version <latest> -d charts/upstream` |
| `kustomizations/metallb-config/ipaddresspool.yaml` | pool `192.168.1.200-192.168.1.230` — the placeholders to tune to the LAN |
| `kustomizations/metallb-config/l2advertisement.yaml` | L2 advertisement for `home-lan` pool |

## Conventions & rules

- helmfile release `metallb` (stage `upstream`) must stay before `metallb-config` (stage `config`).
- IP pool must not overlap the DHCP/static range of the LAN — ask before changing.
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
