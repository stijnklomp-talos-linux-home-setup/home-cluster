<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->

# AGENTS.md (cicd-verdaccio)

Cache-only NPM registry (read-through proxy of registry.npmjs.org) for Tekton TaskRuns. No publishing, no auth, in-cluster service only.

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
| `charts/upstream/verdaccio-4.35.1.tgz` | pinned; refresh: `helm pull verdaccio/verdaccio --version <latest> -d charts/upstream` (repo: `https://charts.verdaccio.org`) |
| `values/verdaccio-values.yaml` | cache-only config: anonymous read, npmjs uplink, `max_users: -1` (no registration), Longhorn PVC 20Gi, ClusterIP |

## Conventions & rules

- Cache-only by design: publish is effectively disabled (no authenticated users). Do not add scoped/private publishing without an ADR.
- TaskRuns reach it at `http://verdaccio.verdaccio.svc.cluster.local:4873` — the per-tenant `npm-npmrc` ConfigMap rendered by `tenant-platform` points there (`developer/pipelines` only renders the `npm-install-v1` task).
- Single replica + RWO PVC: do not scale without an RWX storage class.
- The upstream chart's default config allows publish — the values override must keep `publish: $all` + `max_users: -1` (publish blocked by absence of users).
- Cross-repo rules: workspace root `AGENTS.md`.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- `<ingress-ip>`, `<your-github-org>`, `<region>`, `<user-pool-id>` are TODOs, not values to invent.

## Checklist
- [ ] helmfile chart paths / terragrunt sources resolve (no dangling `./charts/...`)
- [ ] YAML parses (`helm template` or a yaml lint)
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value

## Examples
`cicd-tekton-operator` is the reference helmfile pattern; `tenant-platform` shows the registry-driven approach.

## When stuck
- For pipeline/auth issues: `developer/docs/cicd/pipelines/troubleshoot/` and the restricted wiki runbooks.