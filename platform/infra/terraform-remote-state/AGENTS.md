<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-03 -->

# AGENTS.md (platform/infra/terraform-remote-state)

Bootstrap repo for Terraform state. **State now lives in S3** (bucket `home-terraform-state-<account-id>`, eu-west-1, S3-native locking via `use_lockfile` — no DynamoDB). The bucket is created by this repo's own module.

## Overview
Overview of this component — see `README.md` for prose.

## Setup
| Plans / apply | `terragrunt init` / `terragrunt plan` / `terragrunt apply` (in `prod/`) |


## Commands

| Task | Command |
|------|---------|
| Apply bootstrap unit | `terragrunt init` + `terragrunt apply` in `prod/` |
| Self-migrate (after first apply) | add `remote_state` block to `root.hcl`, then `terragrunt init -migrate-state` in `prod/` |
| Init a consumer repo | `terragrunt init` in its `prod/` (state lands directly in S3) |

## Structure

- `modules/state-store/` — S3 bucket (versioning, SSE-AES256, public-access block); bucket name derived from `aws_caller_identity`.
- `prod/terragrunt.hcl` — unit for the bootstrap apply (local state) and self-migration.
- `root.hcl` — locals (`project`, `region`), `generate versions/providers` (Terraform >= 1.15.9, aws `~> 6.64.0`); `remote_state` added only AFTER the bucket exists.

## Conventions & rules

- Do not commit `*.tfstate` (gitignore covers it).
- The bucket is the single state store for ALL home Terraform repos — consumer `root.hcl` files reference `home-terraform-state-220402652377`; keep them in sync here.
- If the bucket name/region changes, update every `platform/infra/*/root.hcl` + the tekton plan/apply tasks if paths change.
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
