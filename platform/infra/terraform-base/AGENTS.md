<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-03 -->

# AGENTS.md (platform/infra/terraform-base)

Terraform/terragrunt layer of the home setup: S3 state bootstrap + Talos machine configs as code. Monorepo bundling the two former terraform repos; each subdirectory is its own terragrunt project (unit in `prod/`) with its own AGENTS.md.

## Overview
Overview of this repo — see `README.md` for prose.

## Setup (deploy & teardown)

| Task | Command |
|------|---------|
| S3 state bootstrap | `cd terraform-remote-state/prod && terragrunt init` + `terragrunt apply` (one-time; must exist before any other repo stores state) |
| Talos machine configs | `cd terraform-home-talos/prod && terragrunt init` / `terragrunt plan` / `terragrunt apply` |
| Teardown | `terragrunt destroy -auto-approve` per unit, reverse order |

## Commands
Full command table lives in the Setup section; offline validation: `terragrunt plan` (no apply) and `terraform validate` in the unit dir.

## Key files

| Path | Notes |
|------|-------|
| `terraform-remote-state/` | S3 bucket `home-terraform-state-<account-id>` (eu-west-1, versioned, SSE-AES256, S3-native locking — no DynamoDB) |
| `terraform-remote-state/modules/state-store/` | bootstrap module; bucket name derived from `aws_caller_identity` |
| `terraform-home-talos/` | Talos machine configs via `siderolabs/talos` provider v0.11.0; goal: replace manual `home-cluster-1-config/` YAMLs |
| `terraform-home-talos/modules/talos-machine-config/` | `data.talos_machine_configuration` + `talos_machine_configuration_apply` to nodes |

## Conventions & rules

- No root `terragrunt.hcl` — each component dir is a standalone terragrunt project (it is also its own git-tracked unit).
- Order matters: `terraform-remote-state` first (the shared S3 bucket must exist before any other repo runs `terragrunt init`).
- Never commit `.terraform/`, `.terragrunt-cache/`, `*.tfstate` — gitignore covers it.
- Machine secrets (CA material) are NOT wired yet in `terraform-home-talos`; outputs contain real PKI once applied — treat as secrets, never print/commit.
- Component-specific rules live in each subdir's AGENTS.md — read it before editing that component.
- Cross-repo rules: workspace root `AGENTS.md`.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- AWS access goes through the `testing` SSO profile (never static keys); the account id is not a secret.

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
