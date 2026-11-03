# Pipelines setup

Shared Tasks and Pipelines live in `tekton-pipelines`. They are installed once
from `developer/pipelines` and referenced from tenant namespaces with
the [cluster resolver](https://tekton.dev/docs/pipelines/cluster-resolver/).
Every PipelineRun runs **in its tenant namespace** — the namespace of the
Repository CR that matched the webhook.

Tenancy is declarative. The registry
(`platform/cicd/tenant-platform/values/tenants.yaml`)
is the only list of tenants, namespaces and repositories; PaC auto-configure is
off. The operator procedure is the [tenancy runbook](../tenancy/runbook.md); the
contract is the [tenancy spec](../tenancy/spec.md).

## Making a repo Pipelines-as-Code enabled

1. Add the repository to its owner's entry in the registry:

   ```yaml
   - owner: <github-owner>
     namespace: <tenant-namespace>
     tier: build | deploy
     repos:
       - name: <repo-name>
         url: https://github.com/<github-owner>/<repo-name>
   ```

   Apply the tenant platform (`tenant-platform`, stage `tenancy`). The chart
   creates the tenant namespace, quota, network policy, `tekton-bot`,
   `npm-npmrc` and one Repository CR per repository. See
   [Add a tenant](../tenancy/runbook.md#add-a-tenant).

2. The public GitHub App (`pac-talos-linux-home-setup`) must be installed on the
   owner. Webhooks arrive via the gosmee relay — no public IP or domain needed.
   An installation on an owner that is not in the registry is inert.

3. Add `.tekton/pipelinerun.yaml` to the repo with the PaC annotations:

   ```yaml
   metadata:
     annotations:
       pipelinesascode.tekton.dev/on-event: "[pull_request, push]"
       pipelinesascode.tekton.dev/on-target-branch: "[main]"
   ```

4. Reference shared Tasks/Pipelines through the cluster resolver. A bare
   `taskRef.name` only searches the tenant namespace; the shared catalog is in
   `tekton-pipelines`:

   ```yaml
   spec:
     taskRunTemplate:
       serviceAccountName: tekton-bot
       podTemplate:
         securityContext:
           fsGroup: 65532
           fsGroupChangePolicy: OnRootMismatch
     pipelineSpec:
       tasks:
         - name: build
           taskRef:
             resolver: cluster
             params:
               - {name: kind, value: task}
               - {name: name, value: build-push-v1}
               - {name: namespace, value: tekton-pipelines}
   ```

   Use the `taskRef` names from the shared catalog; do not copy shared Task
   definitions into the repository unless the task is genuinely tenant-specific.

5. Push — PaC starts the PipelineRun in the tenant namespace.

### Run namespace

The Repository CR namespace **is** the run namespace. There is no separate PaC
setting. `tekton-pipelines` holds shared Tasks/Pipelines and Tekton
configuration only; it holds no tenant Repository CR. The PaC
`target-namespace` annotation is not a tenancy mechanism — the registry and the
Repository CR namespace decide where a run lands.

## Who can trigger (whitelist)

- **Built-in ACL**: repo owner / collaborator / org member / push permissions.
- **Root `OWNERS` file** (at the repo root, on the default branch) — optional
  per-repo whitelist fallback; PaC does NOT read `.tekton/OWNERS`.
- **No `settings.policy`**: PaC GitHub policy entries are team slugs (org-only)
  and a set policy short-circuits the built-in ACL — on single-user/personal
  repos leave it unset.
- Unauthorized users are blocked with a `Pending` check; an authorized user must
  comment `/ok-to-test <sha>` (SHA required cluster-wide).

## Available resources

All definitions live in `tekton-pipelines`; runs execute in the tenant
namespace.

| Name | Purpose |
|---|---|
| `git-clone-v1` | clone repo into workspace |
| `npm-install-v1` | pnpm/npm install via the Verdaccio cache (`cicd-verdaccio`); bind the `npmrc` workspace to the tenant's `npm-npmrc` ConfigMap |
| `build-push-v1` | rootless Buildah build + push (Docker Hub or the tenant's registry); bind the optional `npmrc` workspace for Dockerfile npm installs |
| `kaniko-v1` | deprecated alias of `build-push-v1`; may be removed once no `.tekton/` references it; it no longer runs kaniko |
| `app-release-v1` | clone → build-push → optional `gitops-yaml-bump-v1`; used by the app-repo PipelineRun examples |
| `gitops-yaml-bump-v1` | sets flat `key=value` keys in a `home-cluster` app declaration and commits (`[skip ci]`) → ArgoCD syncs the env |
| `terragrunt-plan-v1` | plan tf, store on the deploy tenant's `home-terraform-plans` PVC |
| `terragrunt-apply-destroy-v1` | apply a stored plan (restricted) |

## Environments: dev / staging / prod

Copy the PipelineRun examples from
`developer/pipelines/charts/app-release/usage-examples/v1/` into your
repo's `.tekton/` directory (or let `platform/gitops/scripts/new-service.sh` do it).
The examples are starting points: in a tenant namespace, resolve the shared
Pipeline with `pipelineRef.resolver: cluster` (or inline the `pipelineSpec` with
cluster-resolver `taskRef`s) — a bare `pipelineRef.name` only finds Pipelines in
the tenant namespace.

| File | Trigger | Effect |
|---|---|---|
| `ci-pull-request-pipelinerun.yaml` | PR to `main` | build + push the PR image (no GitOps write) |
| `cd-dev-pipelinerun.yaml` | push/merge to `main` | build `:<sha>`, set `imageTagDev` → `<app>-dev` auto-syncs |
| `cd-staging-pipelinerun.yaml` | push a `v*` tag on `main` | build `:<git_tag>`, set staging + prod tag/revision keys → `<app>-staging` auto-syncs, `<app>-prod` waits for a manual Sync |

The app itself is declared once in `home-cluster` (`developer/apps/<cluster>/<app>.yaml`)
— see `developer/docs/architecture-decision-log/home/gitops-environments-adr.md`. Notes:

- Staging/prod pin the app repo to the release tag; dev tracks `main`.
- Tag matching uses `refs/tags/v*`; GitHub may not deliver webhooks when more
  than three tags are pushed at once — push tags in small batches.
- `gitops-pat` (contents:write on `home-cluster`) must exist in the **deploy
  tenant namespace** before the CD pipelines can push. It is operator-managed
  and out-of-band; never commit it.
- `dockerhub-docker-config` is the operator's deploy-tenant push credential. A
  `build` tenant supplies its own registry Secret instead; see
  [Secrets model](../tenancy/runbook.md#secrets-model).
