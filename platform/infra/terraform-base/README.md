# Terraform base layer

Terraform/terragrunt projects backing the cluster: shared state store and Talos machine configs. One component per directory; each is a self-contained terragrunt project (unit in `prod/`).

## Components

| Directory | What it does |
|-----------|--------------|
| `terraform-remote-state` | Bootstrap: one shared S3 state bucket `home-terraform-state-<account-id>` (eu-west-1, S3-native locking, no DynamoDB) |
| `terraform-home-talos` | Talos machine configs (controlplane + worker) as code with the `siderolabs/talos` provider (v0.11.0) |

## Prerequisites

- AWS access via the `testing` SSO profile (no static keys).
- The state bucket must exist before any other terragrunt repo inits (one-time bootstrap below).

## Bootstrap (one-time)

```sh
cd terraform-remote-state/prod
terragrunt init
terragrunt apply   # creates the bucket
```

Then every other terragrunt repo (incl. this one's second unit) just runs `terragrunt init` in its `prod/` — state lands directly in S3.

## Apply

```sh
cd terraform-home-talos/prod
terragrunt init
terragrunt plan
terragrunt apply
```

## Cleanup

```sh
terragrunt destroy -auto-approve
```

per unit, in reverse order.