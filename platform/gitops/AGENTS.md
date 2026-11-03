<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-03 -->

# AGENTS.md (platform/gitops)

ArgoCD orchestration for `home-cluster-1` (namespace `argo`): app-of-apps via
`bootstrap.yaml` plus one ApplicationSet per target cluster
(`applications/<cluster>.yaml`) that expands service declarations into
`<app>-dev` / `-staging` / `-prod` Applications. Bare-metal now, EKS later.

**The declarations are not here** — they are the developer contract and live in
`developer/apps/<cluster>/<app>.yaml`. This directory owns the machinery that
reads them.

## Setup (deploy & teardown)

| Task | Command |
|------|---------|
| Bootstrap (once) | `kubectl apply -f bootstrap.yaml` (app-of-apps → `path: platform/gitops/applications`) |
| Teardown | `kubectl delete application home-apps -n argo` (finalizer prunes generated Applications) |

## Commands

| Task | Command |
|------|---------|
| Add a service | `developer/scripts/new-service.sh --app <app> --repo <url> --image <registry/image> [--chart-path deploy/<app>] [--cluster baremetal\|eks] [--app-repo-dir <dir>]` |
| Validate YAML offline | `docker run --rm -v "$PWD:/ws:ro" -w /ws --entrypoint /bin/sh ghcr.io/helmfile/helmfile:latest -c 'yq -e "." platform/gitops/applications/*.yaml developer/apps/*/*.yaml'` |
| Inspect generated apps | `kubectl get applications -n argo` (container per workspace rules) |
| Release prod | click **Sync** on `<app>-prod` in ArgoCD (or `argocd app sync <app>-prod`) |
| Register EKS | `argocd cluster add <kube-context> --name eks` |

## Key files

| Path | Notes |
|------|-------|
| `bootstrap.yaml` | applied once via kubectl (NOT under `applications/`, so it cannot prune itself); points at `platform/gitops/applications` |
| `applications/baremetal.yaml` | ApplicationSet `apps-baremetal`: matrix (git file generator `developer/apps/baremetal/*.yaml` × list `dev/staging/prod`); `goTemplate`, `requeueAfterSeconds: 60`; env semantics live here |
| `applications/eks.yaml` | future EKS ApplicationSet (`destination.name: eks`); no-op while `developer/apps/eks/` is empty |
| `templates/service-declaration.yaml.tpl` | declaration skeleton for new services |
| `developer/apps/<cluster>/<app>.yaml` | service declaration — flat keys (`app`, `repoURL`, `chartPath`, `valuesFile`, `imageRepository`, `targetRevision<Env>`, `imageTag<Env>`); edited by the Tekton `gitops-yaml-bump-v1` task |

## Conventions & rules

- **One declaration per (service, target cluster).** Adding the file creates 3 Applications; removing it prunes them. Never hand-create per-env Application manifests.
- **Flat declaration keys only.** The git file generator flattens nested YAML, and `gitops-yaml-bump-v1` rewrites `key: value` lines by name. Renaming `targetRevision<Env>`/`imageTag<Env>` breaks the CD pipelines.
- **`repoURL` is the monorepo** for apps in this repo; `chartPath` then points into it (`developer/examples/<app>/deploy/<app>`). An external app keeps its own `repoURL`.
- **Every declaration has `valuesFile`.** The ApplicationSet renders `helm.valueFiles` only when it is non-empty (`""` = chart defaults). The three GraphQL declarations use `values-verification.yaml`.
- **The GraphQL stack is single-environment.** Charts/supergraph/CNPs hardcode `home-users`/`home-orders`/`home-graphql` while the generator creates `<app>-<env>` namespaces. Do not claim multi-env support for these declarations until the charts are templated per env.
- **Env semantics** (dev auto, staging auto, prod manual) are encoded in `applications/<cluster>.yaml`'s list generator — change them in one place.
- **Prod is manually synced** (no `automated` syncPolicy); CI only records the release tag. Manual sync does not prune by default.
- **Chart changes are never hand-applied.** Tekton bumps tags here; ArgoCD deploys from the app chart at the pinned revision.
- `bootstrap.yaml` stays out of the synced path (`applications/`) — otherwise it prunes itself.
- Secrets never live in this repo; ArgoCD creds are credential templates configured in `platform/cicd/cicd-argocd`.
- Cross-repo rules: workspace root `AGENTS.md`.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- The `gitops-pat`/`argocd-github-creds` PATs are cluster Secrets (namespaces `tekton-pipelines`/`argo`) — never paste them into declarations.
- `<ingress-ip>`, `<your-github-org>`, `<region>`, `<user-pool-id>` are TODOs, not values to invent.

## Checklist
- [ ] ApplicationSet `repoURL` + declaration paths resolve (`developer/apps/<cluster>/`)
- [ ] YAML parses (see Commands)
- [ ] Declarations and ApplicationSets changed together when the model changes
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value

## Examples
`developer/apps/baremetal/test-app.yaml` is the reference declaration (external repo); `applications/baremetal.yaml` is the reference ApplicationSet.

## When stuck
- ArgoCD troubleshooting: Application events + `kubectl get applicationset apps-baremetal -n argo -o yaml` (status.conditions).
- Pipeline/auth issues: `developer/docs/cicd/pipelines/troubleshoot/` and the restricted wiki runbooks.
