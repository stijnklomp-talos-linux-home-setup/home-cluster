<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-03 -->

# AGENTS.md (platform/infra/terraform-home-talos)

Manages Talos machine configs (controlplane + worker) as code with the `siderolabs/talos` provider (v0.11.0). Goal: replace manual `home-cluster-1-config/` YAML files over time.

## Overview
Overview of this component — see `README.md` for prose.

## Setup
| Plans / apply | `terragrunt init` / `terragrunt plan` / `terragrunt apply` (in `prod/`) |


## Commands (run inside `prod/`)

| Task | Command |
|------|---------|
| Init / Plan / Apply | `terragrunt init` / `terragrunt plan` / `terragrunt apply` |
| Try config on node | `talosctl --talosconfig <out> -n 192.168.1.107 apply-config --file <mc.yaml> --mode try` |

## Structure

- `root.hcl` — cluster facts in `locals` (endpoint `https://192.168.1.107:6443`, nodes, versions `v1.36.4`/`v1.13.9`), `remote_state` (S3, shared bucket), talos provider generated.
- `modules/talos-machine-config/main.tf` — ported to talos provider 0.11 API: machine configs are `data.talos_machine_configuration` (with `machine_type`); `talos_machine_configuration_apply` applies them to nodes.
- `prod/terragrunt.hcl` — inputs from root locals.

## Conventions & rules

- Machine secrets (CA material) are NOT wired yet — `machine_secrets` is an optional variable; required at apply and must come from secrets management, never committed. Until wired, the module validates/plans only. talosconfig/kubeconfig outputs are deferred to that follow-up (they need `client_configuration` from `talos_machine_secrets`).
- Porting manual tweaks (kubePrism, hostDNS, UKI, quotas, PodSecurity, audit policy) into `config_patches` is a TODO — do not claim full code-managed coverage until done.
- Node IPs/versions in `root.hcl` must match `home-cluster-1-config/` + `platform/node/` — changing them without updating those is a mismatch.
- Outputs contain real PKI once applied — treat as secrets (never print/commit).
- Cross-repo rules: workspace root `AGENTS.md`.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- `<ingress-ip>`, `<your-github-org>`, `<region>`, `<user-pool-id>` are TODOs, not values to invent.

## Checklist
- [ ] helmfile chart paths / terragrunt sources resolve (no dangling `./charts/...`)
- [ ] YAML parses (`helm template` or a yaml lint)
- [ ] 3-way sync files unchanged unless intentionally updated together
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value

## Examples
`home-cluster-internal/terraform/test-terraform-apply-job` is the reference plan/apply CI flow.

## When stuck
- For pipeline/auth issues: `developer/docs/cicd/pipelines/troubleshoot/` and the restricted wiki runbooks.
