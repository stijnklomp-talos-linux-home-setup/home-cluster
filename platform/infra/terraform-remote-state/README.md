# Terraform remote state (S3)

All Terraform applies in the home AWS account share one S3 state bucket, created
by this repo's bootstrap unit.

- Bucket: `home-terraform-state-<account-id>` (versioned, SSE-AES256, public access blocked)
- Locking: S3-native (`use_lockfile = true`) — no DynamoDB table
- Region: `eu-west-1`

## Bootstrap flow (one-time)

```sh
cd prod
terragrunt init           # local state
terragrunt apply          # creates the bucket
# root.hcl then gains the remote_state block (this repo self-migrates)
terragrunt init -migrate-state
terragrunt apply          # no-op, confirms state now lives in S3
```

## Consumers

Every other terragrunt repo (`platform/infra/*`, incl. the internal test job)
wires `remote_state` in its `root.hcl` with a per-unit key
(`<unit-path>/terraform.tfstate`). Run `terragrunt init` there once — no
migration needed unless a local `*.tfstate` already exists.

## Commands

| Task | Command |
|------|---------|
| Init / Plan / Apply | `terragrunt init` / `terragrunt plan` / `terragrunt apply` (in `prod/`) |
| Destroy | `terragrunt destroy -auto-approve` |