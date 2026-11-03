<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-03 | Last verified: 2026-11-03 -->

# AGENTS.md (home-cluster)

Monorepo for the `home-cluster-1` home lab. Two zones: `platform/` (what makes
the cluster run) and `developer/` (what you need to deploy an app). Restricted
material lives in a **separate private repo**, `home-cluster-internal`, cloned
next to this one — never in this tree.

**Precedence:** the **closest `AGENTS.md`** wins. Read it before editing.

## Scoped AGENTS.md

**Platform**
- [platform/node/AGENTS.md](./platform/node/AGENTS.md) — Talos install, upgrades, node patches
- `platform/infra/`: [terraform-base](./platform/infra/terraform-base/AGENTS.md), [terraform-remote-state](./platform/infra/terraform-remote-state/AGENTS.md), [terraform-cognito-federation](./platform/infra/terraform-cognito-federation/AGENTS.md), [terraform-home-talos](./platform/infra/terraform-home-talos/AGENTS.md)
- [platform/cluster/AGENTS.md](./platform/cluster/AGENTS.md) — base layer, plus `home-cilium/`, `home-metallb/`, `home-cert-manager/`, `home-longhorn/`, `home-istio/` (ambient mesh + internal edge Gateway)
- [platform/cicd/AGENTS.md](./platform/cicd/AGENTS.md) — plus `cicd-tekton-operator/`, `cicd-tekton-pipelines-as-code/`, `cicd-verdaccio/`, `cicd-argocd/`, `tenant-platform/` (the tenant registry)
- [platform/gitops/AGENTS.md](./platform/gitops/AGENTS.md) — ApplicationSets, bootstrap, environment model

**Developer**
- [developer/docs/AGENTS.md](./developer/docs/AGENTS.md) — ADRs, access, pipeline docs, tenancy runbook
- [developer/pipelines/AGENTS.md](./developer/pipelines/AGENTS.md) — reusable Tekton tasks/pipelines + usage examples
- [developer/examples/home-users/AGENTS.md](./developer/examples/home-users/AGENTS.md) — reference subgraph (REST/OpenAPI + GraphQL + JWT)
- [developer/examples/home-orders/AGENTS.md](./developer/examples/home-orders/AGENTS.md) — cross-service federation example
- [developer/examples/home-graphql/AGENTS.md](./developer/examples/home-graphql/AGENTS.md) — Hive Gateway, supergraph composition

**Restricted (separate clone, not in this repo)**
- `../home-cluster-internal/wiki/AGENTS.md` (RESTRICTED)
- `../home-cluster-internal/kubernetes/sso-auth-rbac/AGENTS.md`

## Commands (Docker-first — nothing is installed on the host)

`$PWD` is the repo root; live cluster config is a **sibling** directory.

| Task | Command |
|------|---------|
| kubectl (read-only) | `docker run --rm -v "$PWD/../home-cluster-1-config:/cfg:ro" registry.k8s.io/kubectl:v1.36.4 --kubeconfig /cfg/kubeconfig <args>` |
| talosctl (read-only) | `docker run --rm -v "$PWD/../home-cluster-1-config:/cfg:ro" ghcr.io/siderolabs/talosctl:v1.13.9 --talosconfig /cfg/talosconfig <args>` |
| Helm lint/template | `docker run --rm -v "$PWD/<dir>:/src" -w /src alpine/helm:latest lint ./charts/<name>` |
| helmfile (per component) | `docker run --rm --entrypoint /usr/local/bin/helmfile -v "$PWD/<dir>:/src" -v "$PWD/../home-cluster-1-config:/cfg:ro" -e KUBECONFIG=/cfg/kubeconfig -w /src ghcr.io/helmfile/helmfile:latest -l stage=<stage> apply` |
| YAML parse check | `docker run --rm -v "$PWD:/ws:ro" -w /ws --entrypoint /bin/sh ghcr.io/helmfile/helmfile:latest -c 'yq -e "." <file>'` |
| Offline tenancy checks | `cd platform/cicd/tenant-platform && ./tests/render-checks.sh` |

## Zone map

| Path | Contains | Key rules |
|------|----------|-----------|
| `platform/node/` | Talos machine config, patches, schematics | `patches/cni-cilium.yaml` (`cni: none`) is mandatory on every node; `patches/user-namespaces.yaml` + `patches/buildah-seccomp.yaml` are required by the rootless build task |
| `platform/infra/` | Terraform/terragrunt: remote state, Cognito federation, Talos cluster | state in S3 (`home-terraform-state-<account-id>`); run `terragrunt` in `prod/` |
| `platform/cluster/` | Cilium, MetalLB, cert-manager, Longhorn, Istio ambient + edge Gateway | install order Cilium → MetalLB → cert-manager → Longhorn → Istio; ingress-nginx is retired |
| `platform/cicd/` | Tekton operator, PaC, Verdaccio, ArgoCD, tenant-platform | deploy in dependency order; `tenant-platform/values/tenants.yaml` is **the only** tenant list |
| `platform/gitops/` | ApplicationSets, bootstrap, env model | bootstrap applied once via kubectl; CI bumps tags via `gitops-yaml-bump-v1`; prod synced manually |
| `developer/docs/` | ADRs, access, pipelines, ops, tools | docs-only, markdown |
| `developer/pipelines/` | Reusable Tekton tasks/pipelines | installed once, referenced via cluster-resolver from every tenant namespace |
| `developer/apps/` | One declaration per (app, cluster) | flat keys; CI rewrites `imageTag<Env>`/`targetRevision<Env>` by name |
| `developer/examples/` | `home-users`, `home-orders`, `home-graphql` | deployed reference services; `home-users` is the starting point for new services |
| `developer/scripts/` | `new-service.sh` scaffolding | writes `developer/apps/<cluster>/<app>.yaml` |
| `tools/` | Home-dir files (`tools/home/.local/bin/`) | e.g. `agentmemory-proxy.mjs` |

## Boundaries

### Always Do
- Read the closest `AGENTS.md` before editing.
- Read-only cluster commands are always fine (`get`, `list`, `describe`, `logs`, `version`, `health`).
- Run CLIs via containers (commands above) — nothing is installed on the host.
- Parse-check YAML edits before claiming done (`helm template`, `yq`, or a lint).
- Keep the edited sub-project's `AGENTS.md` "Last updated" fresh.
- Treat `platform/cicd/tenant-platform/values/tenants.yaml` as the **only** tenant/namespace list.
- Put new work inside an existing zone directory; never add a top-level directory. If a developer needs it, it goes in `developer/`; if it only keeps the cluster running, `platform/`.

### Ask First
- **Every write/mutating cluster command** (`kubectl apply/delete/scale/exec`, `helmfile apply`, `talosctl apply-config/upgrade/reset/drain/reboot`, ...).
- Applying `tenant-platform` changes that remove a registry entry — the pruner deletes the tenant namespace and its Longhorn PVC data.
- The disruptive Cilium rollout/rollback (all pods restart); follow `platform/cluster/home-cilium/README.md`.

### Never Do
- Commit/paste live credentials from `../home-cluster-1-config/`, real secrets anywhere (`*.example.yaml` + `.gitignore` only), `*.tfstate`, `.terragrunt-cache/`.
- Put restricted `home-cluster-internal` content into this repo — link instead.
- Replace placeholders with fabricated values — no invented IPs, versions, or chart paths.
- Edit vendored `charts/upstream/*.tgz` or `kustomizations/upstream/*.yaml` in place (refresh via the documented `helm pull`/`curl`).
- Relax a tenant namespace's `restricted` Pod Security, or give tenant pods privileged mode/host namespaces/added capabilities, to make a build or pipeline work. Fix the image/task, or change the platform through an ADR.

## Heuristics

| When | Do |
|------|-----|
| Deploying the base layer | Talos `cni: none` patch → **Cilium** → MetalLB → cert-manager → Longhorn → Istio (then the edge Gateway helmfile); CI/CD after base. Full order: `platform/README.md` |
| Tearing down the base layer | Destroy the edge Gateway first (`cd platform/cluster/home-istio && helmfile -f helmfile-edge.yaml.gotmpl destroy`), then `helmfile -l 'stage!=crds' destroy` per dir in reverse order. Plain `helmfile destroy` in `home-istio` also deletes `gateway-api-crds` cluster-wide. Rollback: `developer/docs/operations/istio-ambient-cutover.md` |
| Adding/removing a PaC tenant | Edit `platform/cicd/tenant-platform/values/tenants.yaml`, apply; see `developer/docs/cicd/pipelines/tenancy/runbook.md` |
| Developer asks how to deploy | Point at `developer/README.md`; the copy-source is `developer/examples/home-users` |
| CI/CD pipeline or auth issue | `developer/docs/cicd/pipelines/troubleshoot/` + `developer/docs/cicd/pipelines/tenancy/runbook.md` + the restricted wiki runbooks |
| Unsure about cluster state, credentials, or remote state | Ask the user — never guess |

## When stuck
- Read the component's `README.md` + `AGENTS.md` before the parent's.
