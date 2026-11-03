# Deploy an app on home-cluster-1

Four things and you are running. Everything below is in this directory.

## 1. Get access

You need a kubeconfig from the cluster's SSO (Pinniped + Cognito), and your
tenant namespace must already exist.

- Login + kubeconfig: [`docs/cicd/access/README.md`](docs/cicd/access/README.md)
- Trusting the cluster CA on your machine: [`docs/cicd/access/ca-trust.md`](docs/cicd/access/ca-trust.md)
- No namespace yet? The operator adds you to the tenant registry:
  [`docs/cicd/pipelines/tenancy/runbook.md`](docs/cicd/pipelines/tenancy/runbook.md)

## 2. Make it buildable

Your repo needs a `Dockerfile` and a `.tekton/` directory holding the
PipelineRun definitions that PaC triggers. Copy them from the catalog:

| File to copy | Purpose |
|--------------|---------|
| [`pipelines/charts/app-release/usage-examples/v1/ci-pull-request-pipelinerun.yaml`](pipelines/charts/app-release/usage-examples/v1/ci-pull-request-pipelinerun.yaml) | build on every pull request |
| [`pipelines/charts/app-release/usage-examples/v1/cd-dev-pipelinerun.yaml`](pipelines/charts/app-release/usage-examples/v1/cd-dev-pipelinerun.yaml) | build + bump dev on merge to `main` |
| [`pipelines/charts/app-release/usage-examples/v1/cd-staging-pipelinerun.yaml`](pipelines/charts/app-release/usage-examples/v1/cd-staging-pipelinerun.yaml) | build + record the release on a `v*` tag |

Replace the `TODO` placeholders (`<APP>`, image name). The full catalog of
reusable tasks and pipelines is in [`pipelines/README.md`](pipelines/README.md).

**If your app lives inside this monorepo** (like the examples), its pipelines go
in the repo-root `.tekton/` directory along with everyone else's, so each
PipelineRun also needs a path filter:

```yaml
pipelinesascode.tekton.dev/on-path-change: "[developer/examples/<app>]"
```

Without it, every push would run every app's pipeline.

## 3. Declare it

One file per (app, cluster) in [`apps/`](apps/) — this is what ArgoCD reads:

```sh
developer/scripts/new-service.sh \
  --app my-app \
  --repo https://github.com/<owner>/my-app \
  --image docker.io/<user>/my-app \
  --chart-path deploy/my-app
```

That writes `apps/baremetal/my-app.yaml`. Commit it and ArgoCD generates
`my-app-dev`, `my-app-staging` and `my-app-prod` on its next poll.

The declaration keys are flat and load-bearing: CI rewrites `imageTag<Env>` and
`targetRevision<Env>` by name. Do not rename them. The skeleton is
[`../platform/gitops/templates/service-declaration.yaml.tpl`](../platform/gitops/templates/service-declaration.yaml.tpl).

## 4. Ship it

| Trigger | What happens |
|---------|--------------|
| Pull request | build only (no GitOps bump) |
| Merge to `main` | build → `imageTagDev` bumped → dev auto-syncs |
| Push a `v*` tag | build → staging + prod tags recorded → staging auto-syncs |
| Prod | **manual** — click Sync on `<app>-prod` in ArgoCD |

Rollback is a sync to an older revision; nothing is rebuilt.

## Copy a working example

[`examples/`](examples/) holds three deployed services: `home-users` (a
federated subgraph — the best starting point for a new service), `home-orders`
(cross-service federation) and `home-graphql` (the aggregation gateway). Each has
its chart under `deploy/`, its own docs, and its declaration in `apps/baremetal/`.

## When it does not work

- Pipeline or auth problems: [`docs/cicd/pipelines/troubleshoot/`](docs/cicd/pipelines/troubleshoot/)
- Tenancy, secrets, RBAC: [`docs/cicd/pipelines/tenancy/runbook.md`](docs/cicd/pipelines/tenancy/runbook.md)
- How the platform is put together: [`docs/architecture-decision-log/`](docs/architecture-decision-log/README.md)
- If your app needs something the platform does not offer, that is a `platform/`
  change — open it as a platform change, do not work around the tenant sandbox.
