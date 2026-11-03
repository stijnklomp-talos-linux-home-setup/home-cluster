<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-03 -->

# AGENTS.md (platform/cluster)

Day-0 base layer of the home cluster (helmfile era — everything after this goes through ArgoCD). Each subdirectory is its own helmfile project with its own AGENTS.md.

## Overview
Overview of this repo — see `README.md` for prose.

## Setup (deploy & teardown)

Deploy in dependency order, `helmfile apply` inside each component dir:

| Task | Command |
|------|---------|
| Cilium (CNI) | `cd home-cilium && helmfile apply` (first — requires the Talos `cni: none` patch; see `home-cilium/README.md`) |
| MetalLB | `cd home-metallb && helmfile apply` (after Cilium — nothing else gets an IP without it) |
| cert-manager | `cd home-cert-manager && helmfile apply` |
| Longhorn | `cd home-longhorn && helmfile apply` |
| Istio (ambient) | `cd home-istio && helmfile apply` (after Cilium `cni.exclusive: false`; first install on a fresh cluster: `--skip-diff-on-install`) |
| Edge Gateway | `cd home-istio && EDGE_LB_IP=<lb-ip> EDGE_HOST_SUFFIX=<lb-ip>.sslip.io helmfile -f helmfile-edge.yaml.gotmpl apply` (production edge; separate helmfile because the address/host suffix are environment values) |
| Cleanup | `helmfile -l 'stage!=crds' destroy` per dir, reverse order (Cilium last); destroy the edge Gateway first so it releases the address. Plain `helmfile destroy` in `home-istio` also deletes the Gateway API CRDs — only when abandoning Istio |

## Commands
Full command table lives in the Setup section; offline validation: `helm template <rel> ./charts/<name>` and `helm lint` via the alpine/helm container (see workspace root AGENTS.md).

## Key files

| Path | Notes |
|------|-------|
| `home-cilium/` | Cilium CNI; `verify-isolation.sh` is the tenant egress acceptance test; must be installed before MetalLB |
| `home-metallb/` | L2 LoadBalancer, pool `<lb-ip-range>` (see `ipaddresspool.yaml`; adjust to LAN) |
| `home-cert-manager/` | cert-manager + ClusterIssuers; `home-issuer` internal CA for every LAN ingress |
| `home-longhorn/` | default StorageClass `longhorn`; needed by Tekton workspaces, plans PVC, stateful services |
| `home-istio/` | Istio 1.31.3 + ambient mTLS (no sidecars) + Gateway API CRDs + the production edge Gateway `istio-internal` (separate `helmfile-edge.yaml.gotmpl`); `verify-ambient.sh` proves mTLS success and plaintext denial |

## Conventions & rules

- No root `helmfile.yaml` — each component dir is a standalone helmfile project (it is also its own git-tracked unit).
- Deploy order matters: `home-cilium` first (after the Talos `cni: none` patch), then `home-metallb`, then the rest; `home-istio` needs Cilium's `cni.exclusive: false`, and its edge Gateway is applied last (it takes the MetalLB address that ingress-nginx used to own).
- Component-specific rules live in each subdir's AGENTS.md — read it before editing that component.
- Never edit vendored `charts/upstream/*.tgz` in place; refresh via documented `helm pull`.
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
