---
name: home-cluster-kubectl-access
description: Use when interacting with the home Kubernetes cluster via kubectl using the kubeconfig in `../home-cluster-1-config/`, or when AWS CLI commands are needed for home/AWS infrastructure.
---

# Home Cluster kubectl Access

Defines what the agent may run against the home cluster and AWS. kubectl is allowed with a permission gate for state-changing commands; AWS CLI is user-only.

## When to Use

- Any task touching the home cluster: kubectl get/apply/delete, logs, debugging, helmfile applies against it, cert/secret inspection.
- Any task that needs AWS: state bucket, SSO, ECR, etc.
- Not for other clusters/infrastructure — this skill only covers the home setup.

## kubectl: Allowed Without Asking (read-only)

| Command | Notes |
|---------|-------|
| `get`, `describe`, `logs`, `top`, `explain`, `api-resources`, `version` | free to run |
| `port-forward` | free, but say what you're exposing |
| `apply --dry-run=client`, `kubectl create --dry-run=client -o yaml`, `helm template`, `helm lint` | validation only, no cluster change — free |

## kubectl: ALWAYS Ask the User First (state-changing)

| Command | Notes |
|---------|-------|
| `apply`, `create`, `delete`, `edit`, `patch`, `replace`, `rollout`, `scale`, `drain`, `cordon`, `uncordon`, `taint`, `label`, `annotate`, `helmfile apply`/`install`/`upgrade`/`destroy` | get explicit permission BEFORE running — no "I'll ask after", no "it's a dry run, so it's fine" |
| `exec`, `attach`, `cp`, `debug` | can mutate pods — ask unless clearly diagnostic |

Ask permission even when the change "seems reversible". State the exact command(s) in the permission request. Permission for a command is permission for that command only.

## Using the kubeconfig

- Point kubectl at the file: `kubectl --kubeconfig ../home-cluster-1-config/kubeconfig <cmd>` or `export KUBECONFIG=$PWD/../home-cluster-1-config/kubeconfig`.
- Never edit, print, or commit the kubeconfig (gitignored, contains credentials).

## AWS: User-Only

- The agent NEVER runs `aws` commands. No exceptions, including "just checking".
- The user runs all AWS CLI commands; assume they know how to authenticate (typically an AWS config, e.g. the `testing` SSO profile).
- When AWS is needed, the agent outputs the exact commands for the user to run, with a clear expected result:

  ```sh
  aws --profile testing s3 ls s3://home-terraform-state-<account-id>/
  ```

- Wait for the user's pasted output before drawing conclusions about AWS state — never assume.

## Common Mistakes

| Mistake | Fix |
|---------|-----|
| Running `kubectl delete` "as a dry run" | Dry-run of a destructive intent is still destructive — ask first |
| Running `aws` to "just check" something | Checking is still running AWS — give the command to the user |
| Targeting the wrong cluster | Always pass the kubeconfig flag; verify with `kubectl get nodes` if unsure |
| Printing or committing kubeconfig contents | Treat as a secret |
| Applying helmfile/charts without asking | `helmfile apply` changes cluster state — ask first |