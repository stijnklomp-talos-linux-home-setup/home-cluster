# GitOps (ArgoCD)

ArgoCD orchestration for `home-cluster-1` — the machinery that turns service
declarations into running apps. ArgoCD (namespace `argo`) watches this repo via
the bootstrap Application and syncs the ApplicationSets in `applications/`,
which generate one Application per service and environment.

The declarations themselves are not here: they are the developer contract and
live in [`developer/apps/`](../../developer/apps/).

## Model

One **service declaration** per (service, target cluster):

```
platform/gitops/
  applications/
    baremetal.yaml        # ApplicationSet ← developer/apps/baremetal/*.yaml × [dev, staging, prod]
    eks.yaml              # future EKS target (no-op until the cluster is registered)
  templates/
    service-declaration.yaml.tpl
  bootstrap.yaml          # applied once via kubectl

developer/apps/
  baremetal/<app>.yaml    # declaration (plain YAML, not a k8s manifest)
  eks/<app>.yaml          # future: EKS declarations
```

Adding `developer/apps/baremetal/<app>.yaml` generates `<app>-dev`, `<app>-staging` and
`<app>-prod` (namespace `argo`) on the next ApplicationSet poll. Removing the
file prunes all three. There is no per-env manifest to maintain.

## Environments and triggers

| Environment | Application | Trigger | Sync |
|---|---|---|---|
| dev | `<app>-dev` | merge to `main` → Tekton builds `:<sha>` and sets `imageTagDev` | automatic (prune + selfHeal) |
| staging | `<app>-staging` | push a `v*` tag → Tekton builds `:<git_tag>` and sets `imageTagStaging`/`targetRevisionStaging` | automatic |
| prod | `<app>-prod` | same `v*` release records `imageTagProd`/`targetRevisionProd` | **manual Sync in ArgoCD** (the production gate) |

Namespaces are `<app>-dev` / `<app>-staging` / `<app>-prod` (created by ArgoCD
via `CreateNamespace=true`). Dev tracks the app repo's `main`; staging and prod
pin the release tag, so an environment deploys exactly the commit that was
validated. Prod rollback = sync an older revision from the Application history.

CI → CD is wired by the Tekton `app-release-v1` pipeline (see
`developer/pipelines/charts/app-release/usage-examples/v1/`): the
`gitops-yaml-bump-v1` task commits the new `imageTag*` values here with
`[skip ci]`, and the ApplicationSet re-renders the matching Application.

## Chart overlays (`valuesFile`)

Every declaration carries a flat `valuesFile` key. When non-empty the
ApplicationSet renders it as `helm.valueFiles` (one chart-relative path), so a
per-service overlay such as `values-verification.yaml` survives ArgoCD
self-heal instead of being reconciled away. `""` means the chart defaults.

The three GraphQL declarations (`home-users`, `home-orders`, `home-graphql`)
point at their committed `values-verification.yaml` (test issuer/JWKS). For
production, replace the Cognito `<region>`/`<pool>`/`<client-id>` placeholders
in each chart's `values.yaml` and set `valuesFile: ""` in the declaration.

**Single environment for the GraphQL stack.** These charts and the composed
supergraph use the fixed namespaces `home-users`, `home-orders` and
`home-graphql` (Service DNS, Cilium policies, `supergraph.yaml` routing),
while the ApplicationSet generates `<app>-<env>` namespaces and three
Applications per declaration. Until the stack is templated per environment,
the GraphQL declarations are a single-environment entry, not a real
dev/staging/prod rollout; deploying all three would point three Applications
at the same chart and namespaces. This is documented rather than faked.

**Repo access.** Chart sources and declarations live in the same repo
(`home-cluster`), so one ArgoCD repo credential covers every declaration.
External apps that stay in their own repo need their own credential — the
`github-creds-personal` template already covers personal-account repos.

## Add a service

```sh
# from the repo root (writes developer/apps/<cluster>/<app>.yaml, and optionally
# the app's .tekton/ PipelineRuns)
developer/scripts/new-service.sh --app test-app \
  --repo https://github.com/stijnklomp-LAVE/test-app \
  --image docker.io/stijnklomp/test-app \
  --chart-path deploy/hello-app \
  --app-repo-dir ../test-app
```

Without the script: copy `templates/service-declaration.yaml.tpl` to
`developer/apps/baremetal/<app>.yaml`, fill in the values, and copy the `.tekton/`
PipelineRun examples from
`developer/pipelines/charts/app-release/usage-examples/v1/`.

Declaration keys are flat on purpose: the ApplicationSet git file generator
flattens nested YAML and `gitops-yaml-bump-v1` edits keys by name. Keep
`targetRevision<Env>`/`imageTag<Env>` names as-is.

## Move a service to EKS

1. Register the EKS cluster: `argocd cluster add <context> --name eks`
2. Move (or copy) the declaration from `developer/apps/baremetal/` to `developer/apps/eks/`.
3. ArgoCD prunes the bare-metal Applications and creates the EKS ones.

`applications/eks.yaml` generates nothing while `developer/apps/eks/` is empty.

## Bootstrap

Applied **once** via kubectl (kept out of `platform/gitops/applications/` so it
never prunes itself):

```sh
kubectl apply -f bootstrap.yaml
```

It points at this repo (`path: platform/gitops/applications`, auto-sync +
selfHeal + prune).

## Prerequisites

- ArgoCD repo credentials for `home-cluster` (and for any external app repo)
  — `github-creds` credential template, PAT in namespace `argo`.
- The `gitops-pat` secret in `tekton-pipelines` (contents:write here) for the
  Tekton tag bumps — see `developer/pipelines`.
- App images must be pullable by the cluster (public registry or image pull
  secrets in the env namespaces).
