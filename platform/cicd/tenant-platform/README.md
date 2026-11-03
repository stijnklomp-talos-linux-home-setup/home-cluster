# Tenant platform

One registry entry creates one PaC tenant: a namespace with `restricted` Pod
Security, a quota and LimitRange, a Cilium egress sandbox, the `tekton-bot`
service account, the `npm-npmrc` ConfigMap and one `Repository` CR per
repository. PipelineRuns run **in the tenant namespace**, not in
`tekton-pipelines`.

The specification is `developer/docs/cicd/pipelines/tenancy/spec.md`; the decisions this
component implements are in `developer/docs/architecture-decision-log/home/pac-tenancy-adr.md`
and `cilium-networkpolicy-adr.md`.

```
tenant-platform/
  helmfile.yaml                  release tenant-platform, stage tenancy
  values/tenants.yaml            THE REGISTRY (source of truth)
  charts/tenant-platform/        the platform chart (incl. the admission gate)
  tests/build-tenant-pipelinerun.yaml    live build-tenant acceptance test
  tests/deploy-tenant-pipelinerun.yaml   live deploy-tenant acceptance test
  tests/deploy-tenant-checks.sh          live deploy-tenant placement/isolation checks
  tests/admission-checks.sh      live admit/deny checks for the Repository gate
  tests/render-checks.sh         offline render/validation checks
```

## Registry

`values/tenants.yaml` is the only list of tenants, namespaces and repositories.
PaC auto-configure is disabled, so a repository that is not in the registry
never runs.

```yaml
tenants:
  - owner: <github-owner>          # required, unique (case-insensitive)
    namespace: <dns-1123-label>    # required, unique, explicit alias; run namespace
    tier: build | deploy           # required
    developers:                    # optional; read-only visibility
      - <developer-email>
    repos:                         # required, may be empty
      - name: <dns-subdomain>      # unique per namespace; Repository CR name
        url: <https://github.com/owner/repo>
    concurrencyLimit: 2            # optional, default 2
```

`namespace` is never derived from `owner` at runtime. Uppercase GitHub logins
need an explicit lowercase alias: `stijnklomp-LAVE` → `stijnklomp-lave`.

The chart fails the render with every problem at once for: duplicate owner
(case-insensitive), duplicate namespace, a namespace that is not a DNS-1123
label, duplicate repo name or URL within a namespace, a repo name that is not a
DNS subdomain, a tier other than `build`/`deploy`, a non-positive
`concurrencyLimit`, an owner longer than 63 characters (label-value limit) and
developer emails that collide after slugging.

Per-tenant `quota` and `limitRange` maps override the tier defaults from
`charts/tenant-platform/values.yaml`.

## Tiers

| | `build` | `deploy` |
|---|---|---|
| Kubernetes API egress | denied | allowed (`toEntities: kube-apiserver`) |
| Deploy credentials | none | as required by the GitOps path |
| Registry credentials | tenant-supplied, out of band | operator-supplied |
| Pod Security | `restricted` | `restricted` |
| Developer role | read-only | read-only |

### The deploy tenant (operator org)

The operator organization is registered as one `deploy` tenant. The tier is a
property of the owner, so every repository under the org shares it:

```yaml
- owner: stijnklomp-talos-linux-home-setup
  namespace: stijnklomp-talos-linux-home-setup
  tier: deploy
  developers: [stijnklomp1@hotmail.com]
  repos:  # all 17 org repositories, name = repo basename
    - name: home-cluster
      url: https://github.com/stijnklomp-talos-linux-home-setup/home-cluster
    # ...
  concurrencyLimit: 2
```

The namespace name is an explicit alias of the owner (it happens to be a valid
DNS-1123 label). Applying the registry creates the namespace, the deploy-tier
quota/LimitRange, a `tenant-sandbox` Cilium policy with `toEntities:
[kube-apiserver]`, `tekton-bot`, `npm-npmrc`, the 17 Repository CRs and the
developer bindings.

**Secret model.** The namespace is the boundary: any Secret in it is readable by
that tenant's PipelineRuns (secret mounts bypass RBAC). The deploy tenant
therefore owns the deploy credentials; the build tenant gets none.

| Object | Owner | Where |
|---|---|---|
| `dockerhub-docker-config` | `developer/pipelines` (`docker-build-publish-secrets`) | deploy tenant namespace |
| `home-terraform-plans` PVC + `home-terraform-plans-cleanup` CronJob | `developer/pipelines` (`terragrunt-secrets`) | deploy tenant namespace |
| `gitops-pat`, `cloud-creds`, optional `npm-secret` | operator, out of band | deploy tenant namespace (examples only in Git) |

The shared charts read the namespace from
`developer/pipelines/values/platform.yaml`:

```yaml
namespace: stijnklomp-talos-linux-home-setup
```

`gitops-pat` and `cloud-creds` **do not exist live yet**. The operator creates
them after the tenant is applied (the committed files are `*.example.yaml`
only; never commit the real values):

```sh
kubectl -n stijnklomp-talos-linux-home-setup create secret generic gitops-pat \
  --from-literal=username=tekton-bot \
  --from-literal=password='<GITHUB_PAT>'

kubectl -n stijnklomp-talos-linux-home-setup create secret generic cloud-creds \
  --from-literal=CLOUD_ACCESS_KEY_ID='<ACCESS_KEY_ID>' \
  --from-literal=CLOUD_SECRET_ACCESS_KEY='<SECRET_ACCESS_KEY>'
```

The plan PVC is per-deploy-tenant now. The old `tekton-pipelines` PVC was
deleted during cutover; plan files are ephemeral handoffs and were not copied.
Only one plan+apply pair runs at a time per deploy tenant (RWO volume).

### Cutover and rollback

Order matters: PaC enforces globally unique Repository URLs, so the old CRs must
be gone before the registry creates the new ones.

```sh
# 1. delete the old single-namespace Repository CRs
kubectl delete repositories.pipelinesascode.tekton.dev -n tekton-pipelines --all

# 2. apply the registry (creates the org namespace + 17 Repository CRs)
helmfile -l stage=tenancy sync            # in this directory

# 3. move secrets and plan storage into the org namespace
helmfile --values values/platform.yaml apply   # in developer/pipelines
```

When the shared releases changed namespace, Helm removed the old
`tekton-pipelines/dockerhub-docker-config` Secret and the old
`tekton-pipelines/home-terraform-plans` PVC (and its Longhorn volume). That is
expected: plan files are ephemeral and the Secret is recreated from the
gitignored chart file in the new namespace.

Rollback: keep the pre-tenancy path recoverable until the tenant runs have been
observed for one full pipeline cycle. The ticket #8 retirement is a Git change,
so revert the retirement commit (or check out the pre-tenancy baseline) to get
the `pipelines-as-code-repositories` chart and the shared `tekton-bot` chart
back, then re-add their helmfile releases and apply. The old Repository CRs were
already deleted from `tekton-pipelines`; recreate them from the reverted chart.
Because the admission gate denies Repository CRs in unlabeled namespaces,
disable (delete) the `tenant-platform-repository-owner` ValidatingAdmissionPolicy
and binding first — and re-enable them as soon as the rollback is done. Rolling
back only the org entry is simpler: remove it from the registry, apply (the
pruner deletes the namespace), then revert the shared values/charts and
re-apply.

## Repository admission gate

The registry is the primary control: tenants cannot create Repository CRs, and
only the platform does. A cluster-wide `ValidatingAdmissionPolicy`
`tenant-platform-repository-owner` (plus its binding) is defense-in-depth: it
binds every Repository CR to the tenant namespace it lives in. It matches
`pipelinesascode.tekton.dev/v1alpha1` `repositories` on CREATE and UPDATE with
`failurePolicy: Fail` and `validationActions: [Deny]`.

The rule:

- the owner is the fourth `/`-segment of `spec.url`
  (`https://github.com/<owner>/<repo>` → `<owner>`);
- the namespace must carry the `home.cluster/tenant` label;
- the URL owner and the label value must match case-insensitively
  (`lowerAscii`; GitHub logins are ASCII) — the label keeps the owner verbatim,
  so `stijnklomp-LAVE` satisfies `home.cluster/tenant: stijnklomp-LAVE`;
- a URL with fewer than four segments, no URL, or no label is denied;
- the `pipelines-as-code` namespace is exempt: a global Repository CR there would have no
  tenant namespace.

A mismatch fails like this (the VAP, not the PaC webhook, is the rejecter):

```text
The repositories "admission-deny-owner" is invalid: : ValidatingAdmissionPolicy
'tenant-platform-repository-owner' with binding 'tenant-platform-repository-owner'
denied request: Repository spec.url owner must match the namespace label
home.cluster/tenant (case-insensitive). Expected
https://github.com/<home.cluster/tenant>/<repo> in a tenant namespace; the
pipelines-as-code namespace is exempt for an optional global Repository CR (none exists today).
```

Consequences:

- A Repository CR in `tekton-pipelines` (no tenant label) can no longer be
  created or updated. The pre-tenancy CRs were removed in ticket #8; the gate
  keeps the retired single-namespace path from coming back. Status subresource
  writes are not matched, so the controller is unaffected.
- A tenant cannot point a CR at another owner's repository, even if a future
  tool creates CRs automatically.

Live checks (`tests/admission-checks.sh`) create and delete throwaway
`admission-*` CRs; they never touch the registry-managed CRs:

```sh
KUBECONFIG_DIR=/path/to/home-cluster-1-config \
  ./tenant-platform/tests/admission-checks.sh
```

They assert: matching owner (mixed case) admitted; `pipelines-as-code` exempt;
mismatched owner denied; namespace without the label denied; UPDATE changing the
owner denied; `stijnklomp-lave/test-app` untouched.

**Rollback:** delete the binding and policy (`kubectl delete
validatingadmissionpolicybinding,validatingadmissionpolicy
tenant-platform-repository-owner`) or revert this chart. Re-enabling PaC
auto-configure is a separate PaC change and is not required for the gate.

PaC auto-configure is disabled in
`cicd-tekton-pipelines-as-code/kustomizations/pipelines-as-code/patches/configmaps.yaml`
(`auto-configure-new-github-repo: "false"`, no template keys). An unregistered
owner that installs the GitHub App therefore produces no namespace, no
Repository CR and no PipelineRun.

## What this chart owns vs what PaC owns

| This chart | PaC / Kubernetes / other repo |
|---|---|
| `Namespace` + labels + PSA | PipelineRuns, TaskRuns, pods and PVCs in the namespace |
| `ResourceQuota`, `LimitRange` | Per-run git auth Secret created by PaC |
| `CiliumNetworkPolicy` `tenant-sandbox` | Namespace's auto-created `default` SA and `kube-root-ca.crt` ConfigMap |
| `ServiceAccount` `tekton-bot` (no token automount, no RoleBinding) | `tekton-developer` / `tekton-namespace-lister` ClusterRoles (SSO/RBAC repo, ticket #6) |
| `ConfigMap` `npm-npmrc` | Shared Tasks/Pipelines in `tekton-pipelines` (cluster resolver) |
| One `Repository` CR per `repos[]` (`source` provenance) | GitHub App installation and webhooks |
| Developer `RoleBinding`s + deduped `tekton-namespace-lister` CRBs | |
| Pre-upgrade prune hook + its RBAC | |

A `build` tenant namespace must never contain operator credentials (Docker Hub,
GHCR, AWS, GitOps). `deploy` tenant namespaces hold them by design. Secret
mounts bypass RBAC: any Secret in the namespace is readable by that tenant's
pipelines. This chart renders no Secret itself; the shared charts place the
deploy secrets in the deploy tenant namespace.

Kubernetes creates two objects in every namespace that this chart cannot
suppress: the `default` ServiceAccount and the `kube-root-ca.crt` ConfigMap.
Neither carries a token or a secret; PipelineRun pods use `tekton-bot`, which
has `automountServiceAccountToken: false`.

## Developer visibility (read-only)

`developers:` produces one `RoleBinding` per developer per tenant namespace
(role `tekton-developer`) and one deduped `ClusterRoleBinding` per unique
developer (role `tekton-namespace-lister`) for the dashboard namespace
dropdown. Both ClusterRoles are cluster-scoped and owned by the SSO/RBAC repo
(`home-cluster-sso-auth-rbac/charts/cluster-rbac`, ticket #6); this chart never
defines them.

A developer sees the namespace's PipelineRuns, TaskRuns, Pipelines, Tasks,
CustomRuns, Repository CRs, pods, pod logs and events. The binding is read-only
on purpose: secrets/configmaps stay invisible, and `create` in a namespace is
equivalent to that namespace's workload identity. Viewer identity (the
impersonated email) and workload identity (the `tekton-bot` account a
PipelineRun runs as) are independent — a developer's Role does not constrain
what their pipeline can mount.

The live acceptance check for these bindings is
`home-cluster-sso-auth-rbac/tests/developer-rbac-checks.sh`; run it after
applying this chart.

## Add a tenant

1. Add the registry entry. Use the explicit namespace alias.
2. Install the public GitHub App on the owner (the registry gate makes unknown
   installations inert, but the App must be installed for webhooks).
3. Apply:

   ```sh
   # from the workspace root (Docker-first, nothing installed on the host)
   docker run --rm \
     -v "$PWD/platform/cicd/tenant-platform:/src" \
     -v "$PWD/home-cluster-1-config:/cfg:ro" \
     -e KUBECONFIG=/cfg/kubeconfig \
     -w /src ghcr.io/helmfile/helmfile:latest \
     -l stage=tenancy apply
   ```

4. Push to the tenant repository (or open a PR) and confirm the PipelineRun
   lands in the tenant namespace:
   `kubectl get pipelinerun -n <namespace>`.

Adding a repository to an existing tenant is only a registry edit + apply.

## Remove a tenant — destructive

1. Delete the registry entry.
2. Apply the same command. A **post-install/post-upgrade hook** lists namespaces labelled
   `home.cluster/managed=true`, deletes those whose name is not in the registry
   (`kubectl delete namespace --wait=false`), and deletes
   `tekton-namespace-lister-*` ClusterRoleBindings whose user is no longer in
   the deduped developer set. It then lets Helm apply the new state.
3. Confirm: `kubectl get ns -l home.cluster/managed=true`.

> **WARNING — data loss.** Deleting a namespace deletes its PVCs. Longhorn
> volumes created with the default reclaim policy `Delete` are removed with the
> PVC, so workspace/plan data is **unrecoverable**. If any data must survive,
> back it up or set the PV reclaim policy to `Retain` before applying. There is
> no undo in this chart.

Removing the last tenant is a deliberate action: with an empty registry the
pruner refuses to run (a partial values file would otherwise delete every
tenant). Set `pruner.allowEmptyRegistry=true` for that one apply if you really
mean it.

`helmfile destroy` in this directory is equally destructive: Helm deletes the
tracked Namespaces and with them every PVC/Longhorn volume. Back up data first.

The prune hook's `home.cluster/managed=true` label and the
`tekton-namespace-lister-` CRB prefix are reserved. Do not label unrelated
namespaces with `home.cluster/managed=true`.

## Acceptance test

The live test is `tests/build-tenant-pipelinerun.yaml`. It runs in
`stijnklomp-lave`, clones `stijnklomp-LAVE/test-app` with `git-clone-v1` from
`tekton-pipelines` via the cluster resolver, then runs an inline restricted
probe task that asserts:

- allowed: cluster DNS, Verdaccio (`verdaccio.verdaccio.svc.cluster.local:4873`),
  public internet (`registry.npmjs.org`);
- denied: Kubernetes API (`kubernetes.default.svc.cluster.local:443`), home LAN
  (`192.168.1.107:6443`) and another namespace's service
  (`tekton-dashboard.tekton-pipelines.svc.cluster.local:9097`).

It uses an `emptyDir` workspace, so it does not need a PVC. It deliberately does
not use any other shared task: ticket #3 is what makes the shared tasks
`restricted`-compatible.

```sh
# from the cicd repo root (home-cluster-kubernetes-cicd)
kubectl delete pipelinerun -n stijnklomp-lave tenant-acceptance --ignore-not-found
kubectl apply -f tenant-platform/tests/build-tenant-pipelinerun.yaml
kubectl wait --for=condition=Succeeded --timeout=20m \
  -n stijnklomp-lave pipelinerun/tenant-acceptance
kubectl logs -n stijnklomp-lave \
  -l tekton.dev/pipelineRun=tenant-acceptance --all-containers --prefix
```

The old `tekton-pipelines/test-app` Repository CR was removed during cutover, so
the repository URL matches exactly one CR.

> **Retired single-namespace path:** the `pipelines-as-code-repositories` chart
> and its Helm release are gone (chart deleted from Git in ticket #8, release
> uninstalled), so re-applying the PaC operator/gosmee stages cannot recreate
> `tekton-pipelines/test-app`. The admission gate also blocks creating a
> Repository in an unlabeled namespace such as `tekton-pipelines`.

### Control-plane and clean-rebuild checks (ticket #8)

`tests/retire-single-namespace-checks.sh` asserts the control plane is clean:
`tekton-pipelines` has no Repository CRs, `tekton-bot` SA/Role/RoleBinding, no
`npm-npmrc`, no deploy secrets and no `home-terraform-plans` PVC/CronJob, while
the shared Tasks/Pipelines and Tekton config/dashboard remain; both obsolete
Helm releases are gone; both tenants still have every registry object; and no
`tekton-bot`/`npm-npmrc` exists outside a tenant namespace.

```sh
KUBECONFIG_DIR=/path/to/home-cluster-1-config \
  ./tenant-platform/tests/retire-single-namespace-checks.sh
```

`tests/clean-rebuild-checks.sh` proves the registry is the source of truth. It
refuses to run unless the control-plane checks pass, deletes **only** the build
tenant namespace `stijnklomp-lave`, rebuilds with
`helmfile -l stage=tenancy sync`, asserts every tenant object is back (namespace
labels, quota, LimitRange, CNP, SA without token automount, `npm-npmrc`,
Repository CR, developer RoleBinding + namespace-lister CRB), and re-runs the
acceptance PipelineRun. The org namespace and its real deploy secret/PVC are
never touched.

```sh
KUBECONFIG_DIR=/path/to/home-cluster-1-config \
  ./tenant-platform/tests/clean-rebuild-checks.sh
```

If the probe is denied the allowed targets, check that Cilium is the CNI and the
`CiliumNetworkPolicy` is accepted (`kubectl get cnp -n stijnklomp-lave`); if the
pod is rejected, check the PSA events and whether the shared task is
`restricted`-compatible.

### Deploy tenant acceptance

`tests/deploy-tenant-pipelinerun.yaml` runs in
`stijnklomp-talos-linux-home-setup` under `restricted` PSA and asserts, with one
inline probe task:

- reachable: Kubernetes API (`https://kubernetes.default.svc/` — any HTTP
  response such as 401/403 proves network reachability), cluster DNS, Verdaccio
  and the public internet;
- the namespace has the deploy-tier labels, quota, LimitRange and CNP with
  `kube-apiserver` egress.

`tests/deploy-tenant-checks.sh` wraps the live assertions around it:

```sh
KUBECONFIG_DIR=/path/to/home-cluster-1-config \
  ./tenant-platform/tests/deploy-tenant-checks.sh
```

It also asserts: exactly 17 Repository CRs in the org namespace, 1 in
`stijnklomp-lave`, 0 in `tekton-pipelines`; `dockerhub-docker-config` is in the
org namespace and no deploy secret is in the build namespace; the build
tenant's `tekton-bot` cannot read the org secrets (`kubectl auth can-i`); the
plan PVC/CronJob moved out of `tekton-pipelines`.

Residual: the GitHub webhook round-trip (App installation → PaC → check run)
cannot be exercised from this environment. The PipelineRuns above simulate the
PaC-created run; the webhook path is verified by pushing to the repository.

## Prerequisites

- Cilium installed with the `CiliumNetworkPolicy` CRD. On Flannel the policy is
  accepted and ignored.
- `tekton-pipelines` exists (Tekton operator applied) — the pruner SA/Job and the
  shared Tasks live there.
- Tekton Pipelines has `set-security-context: true` (set in `cicd-tekton-operator`
  in this repo for Pipelines v1.15). Without it, Tekton's injected init/sidecar
  containers are rejected by `restricted` PSA before a task starts.
- Shared task images/steps are `restricted`-compatible (ticket #3).

## Offline validation

```sh
./tenant-platform/tests/render-checks.sh
```

Runs `helm lint`, `helm template`, `helmfile template`, YAML parse checks,
registry-validation failure cases, per-object assertions for the real registry
and a two-tenant fixture, and a stub-`kubectl` test of the prune hook logic.
Nothing is installed on the host and no cluster access is used.

## Layout and ownership

| Object | Name | Scope |
|---|---|---|
| `Namespace` | registry `namespace` | cluster |
| `ResourceQuota` | `tenant-quota` | tenant |
| `LimitRange` | `tenant-limits` | tenant |
| `CiliumNetworkPolicy` | `tenant-sandbox` | tenant |
| `ServiceAccount` | `tekton-bot` | tenant |
| `ConfigMap` | `npm-npmrc` | tenant |
| `Repository` | registry `repos[].name` | tenant |
| `RoleBinding` | `tekton-developer-<email-slug>` | tenant (only if developers) |
| `ClusterRoleBinding` | `tekton-namespace-lister-<email-slug>` | cluster (deduped) |
| `ServiceAccount` / `ClusterRole` / `ClusterRoleBinding` | `tenant-platform-pruner` | `tekton-pipelines` / cluster |
| `ValidatingAdmissionPolicy` + binding | `tenant-platform-repository-owner` | cluster |
| `Job` | `tenant-platform-prune` (post-install/post-upgrade hook) | `tekton-pipelines` |

Helm stores the release in the kubeconfig default namespace (as the other PaC
charts do); every object declares its own namespace.

## Related docs

Operator runbook (add/remove/offboard a tenant, secrets, RBAC, troubleshooting, rollback):
[`developer/docs/cicd/pipelines/tenancy/runbook.md`](https://github.com/stijnklomp-talos-linux-home-setup/home-cluster/blob/main/developer/docs/cicd/pipelines/tenancy/runbook.md).
