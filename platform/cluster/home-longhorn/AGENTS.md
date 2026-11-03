<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-01 -->

# AGENTS.md (home-longhorn)

Day-0 base component: default StorageClass `longhorn` on the worker node (`/dev/sdb`). Needed by Tekton workspaces, the plans PVC, and any stateful service.

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
| `charts/upstream/longhorn-1.12.1.tgz` | pinned; requires K8s >= 1.34 (cluster is 1.35.2) |
| `values/longhorn-values.yaml` | `defaultReplicaCount: 2` (3 workers), tolerates the CI taint so TaskRun volumes attach on the Dell |

## Conventions & rules

- `defaultReplicaCount: 2` — 3 workers (2 Lenovos + Dell CI node), survives any single worker loss. Storage lives on workers only; the control plane stays kube-system only.
- The CI toleration in values is required: TaskRun pods run on the tainted Dell node and their workspace volumes must attach there.
- Backups are not configured; do not claim durability.
- Changing the storage class name breaks every `storageClassName: longhorn` reference across the workspace — ask first.
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
