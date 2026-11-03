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

- `root.hcl` — cluster facts in `locals` (endpoint `https://192.168.1.107:6443`, nodes, versions `v1.36.4`/`v1.13.9`), `remote_state` (S3, shared bucket), talos provider generated. The state key includes the component path because bare `path_relative_to_include()` (`prod`) collides with `terraform-remote-state/prod` in the same bucket. (`terraform-cognito-federation` is safe: its units live in `prod/cognito` and `prod/identity-center`.) The old `prod/terraform.tfstate` object belongs to `terraform-remote-state` — do not migrate it into this unit.
- `modules/talos-machine-config/main.tf` — ported to talos provider 0.11 API: machine configs are `data.talos_machine_configuration` (with `machine_type`); `talos_machine_configuration_apply` applies them to nodes.
- `prod/terragrunt.hcl` — inputs from root locals.

## Conventions & rules

- `talos_machine_secrets.this` holds the live CA material (imported from the now-deleted `home-cluster-1-config/secrets.yaml`). Terraform state is the **only** copy — never let an unimported `apply` create it, and never run `talosctl gen secrets` expecting a recovery path.
- talosconfig/kubeconfig come from `talos_client_configuration` / `talos_cluster_kubeconfig` data sources, so `plan` needs LAN access to the control plane node. Both outputs are sensitive.
- The Talos v1.13 contract defaults already cover kubePrism, hostDNS, UKI cmdline, quotas, PodSecurity, audit policy and the control-plane label. `config_patches` only add the installer image + control-plane install disk. Re-check this against the live config before claiming full coverage.
- `talos_machine_configuration_apply` is intentionally not wired. Do not add it without user approval; validate with `--mode try` first.
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
