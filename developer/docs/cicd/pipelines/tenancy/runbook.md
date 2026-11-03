# PaC tenancy — operator runbook

How to add, operate and remove Pipelines-as-Code (PaC) tenants on
`home-cluster-1`. The contract is [spec.md](./spec.md); the decisions are
[pac-tenancy-adr.md](../../../architecture-decision-log/home/pac-tenancy-adr.md)
and
[cilium-networkpolicy-adr.md](../../../architecture-decision-log/home/cilium-networkpolicy-adr.md).
Vocabulary: [GLOSSARY.md](../../../GLOSSARY.md).

The registry is
`platform/cicd/tenant-platform/values/tenants.yaml`.
There is no other list of tenants, namespaces or repositories.

Commands below use `kubectl` and `talosctl` with `KUBECONFIG`/`--talosconfig`
pointing at `home-cluster-1-config/`. In this workspace both run in containers;
see the workspace `AGENTS.md` for the wrapper commands.

## Model

| Concept | Meaning | Where it lives |
|---|---|---|
| **Tenant** | A GitHub owner (personal account or organization) admitted to CI. The owner is the unit of isolation, authorization and rate limiting. | Registry `owner` |
| **Tenant namespace** | The namespace allocated to one tenant. It is also the **run namespace**: PaC creates the PipelineRun in the namespace of the Repository CR that matched the event. | Registry `namespace`; one `Namespace` per entry |
| **Registry** | The single declarative list of owners, namespaces, tiers, repositories and developer emails. Git is the source of truth; PaC auto-configure is off. | `tenant-platform/values/tenants.yaml` |
| **Tier** | The capability level of the owner: `build` or `deploy`. It is a property of the owner, not of a repository. | Registry `tier` |
| **Repository CR** | The PaC `Repository` object that registers one git repository. Its namespace is the run namespace. | One per registry `repos[]` entry, in the tenant namespace |
| **GitHub App** | One **public** App (`pac-talos-linux-home-setup`) and one PaC controller for every tenant. An installation is one owner's authorization grant; PaC mints a token per installation. | App settings + `pipelines-as-code` namespace |
| **Admission gate** | A `ValidatingAdmissionPolicy` that requires a Repository CR's URL owner to match its namespace's `home.cluster/tenant` label. Defense-in-depth; the registry is the primary control. | Cluster-wide, `tenant-platform-repository-owner` |

Consequences of the model:

- A repository that is not in the registry never runs. Installing the App on an
  unregistered owner creates no namespace, no Repository CR and no PipelineRun.
- `tekton-pipelines` is the control-plane namespace: shared Tasks/Pipelines,
  Tekton configuration and platform objects only. It holds no tenant
  Repository CR.
- A tenant cannot create Repository CRs; only the platform does. Tenant
  developers are read-only.

## Prerequisites

Apply these before the first tenant, and after a cluster rebuild or a new node.

| Prerequisite | Why | Check |
|---|---|---|
| Cilium is the CNI and the `CiliumNetworkPolicy` CRD exists | A policy on Flannel is accepted and ignored | `kubectl get crd ciliumnetworkpolicies.cilium.io`; `kubectl -n kube-system get ds cilium` |
| Tekton operator + Pipelines are installed | Shared Tasks/Pipelines and the cluster resolver | `kubectl get tektonconfig config` |
| `tekton-pipelines` exists | Pruner ServiceAccount/Job and shared catalog | `kubectl get ns tekton-pipelines` |
| Tekton `set-security-context: true` | Tekton's injected init/sidecar containers otherwise fail `restricted` PSA | `kubectl get tektonconfig config -o jsonpath='{.spec.pipeline.set-security-context}'` → `true` |
| `coschedule: disabled` | A TaskRun may bind more than one PVC (terragrunt plan/apply) | `kubectl get tektonconfig config -o jsonpath='{.spec.pipeline.coschedule}'` → `disabled` |
| `tekton-developer` / `tekton-namespace-lister` ClusterRoles | Developer bindings grant nothing without them; owned by the SSO/RBAC repo | `kubectl get clusterrole tekton-developer tekton-namespace-lister` |
| Talos user namespaces | Rootless Buildah fails at `unshare` without it | `talosctl read /proc/sys/user/max_user_namespaces -n <node>` → `15000` |
| Talos `buildah-userns` seccomp profile | `RuntimeDefault` blocks the namespace/mount syscalls rootless Buildah needs | `talosctl ls /var/lib/kubelet/seccomp/profiles -n <node>` → `buildah-userns.json` |

The two Talos patches are
[`platform/node/patches/user-namespaces.yaml`](https://github.com/stijnklomp-talos-linux-home-setup/home-cluster/blob/main/platform/node/patches/user-namespaces.yaml)
(immediate) and
[`platform/node/patches/buildah-seccomp.yaml`](https://github.com/stijnklomp-talos-linux-home-setup/home-cluster/blob/main/platform/node/patches/buildah-seccomp.yaml)
(`--mode=staged`, effective after a reboot). Every node that can run a tenant
PipelineRun needs both. The seccomp file cannot be applied in immediate mode:
stage it and reboot in a maintenance window, or install the file out-of-band on
nodes that have not rebooted.

## Add a tenant

Order matters only for the first push: the registry entry can be applied before
or after the App installation. Unknown installations are inert.

### 1. Add the registry entry

Edit `tenant-platform/values/tenants.yaml`:

```yaml
tenants:
  - owner: <github-owner>          # GitHub login, verbatim (case-insensitive unique)
    namespace: <dns-1123-label>    # explicit alias; unique; the run namespace
    tier: build | deploy           # required
    developers:                    # optional; read-only visibility
      - <developer-email>
    repos:                         # required; may be empty
      - name: <repo-name>          # Repository CR name; unique per namespace
        url: https://github.com/<github-owner>/<repo-name>
    concurrencyLimit: 2            # optional; default 2
```

Rules:

- `namespace` is never derived from `owner` at runtime. Uppercase logins need an
  explicit lowercase alias: `stijnklomp-LAVE` → `stijnklomp-lave`.
- `repos[].name` is both the Repository CR name and the PaC repository name;
  keep it to a DNS subdomain and unique within the tenant.
- `tier` is a property of the owner. Every repository under an organization
  shares the organization's tier.
- `developers` is optional. Emails are personal data; the registry is public, so
  only add an address the developer agrees to publish.

Validate the render before touching the cluster:

```sh
# from the home-cluster-kubernetes-cicd repo root
./tenant-platform/tests/render-checks.sh
```

### 2. Apply the registry

From the workspace root (Docker-first; nothing is installed on the host):

```sh
docker run --rm \
  -v "$PWD/platform/cicd/tenant-platform:/src" \
  -v "$PWD/home-cluster-1-config:/cfg:ro" \
  -e KUBECONFIG=/cfg/kubeconfig \
  -w /src --entrypoint /usr/local/bin/helmfile \
  ghcr.io/helmfile/helmfile:latest \
  -l stage=tenancy apply
```

The image has no entrypoint, so pass `/usr/local/bin/helmfile` explicitly
(otherwise Docker treats `-l` as the command).

The chart creates, for the new entry:

- `Namespace` with `home.cluster/tenant=<owner>`, `home.cluster/tier=<tier>`,
  `home.cluster/managed=true` and `restricted` PSA labels;
- `ResourceQuota tenant-quota` and `LimitRange tenant-limits`;
- `CiliumNetworkPolicy tenant-sandbox`;
- `ServiceAccount tekton-bot` with no token automount and no RoleBinding;
- `ConfigMap npm-npmrc`;
- one `Repository` CR per `repos[]` entry with `source` provenance and the
  registry concurrency limit;
- one `RoleBinding` per developer and one deduped `tekton-namespace-lister`
  ClusterRoleBinding per unique email.

### 3. Install the public GitHub App on the owner

The App is `pac-talos-linux-home-setup`
(`https://github.com/apps/pac-talos-linux-home-setup`). It is public because a
private App can only be installed on its owner account.

1. Open the App page → **Install** (or **Configure** if already installed).
2. Select the owner (personal account or organization). For an organization,
   an org owner may need to approve the installation.
3. Choose **All repositories** or select the repositories that are in the
   registry. Choosing all is fine: the registry, not the installation, decides
   which repositories run, and an unregistered repository is inert.
4. Confirm the installation appears under **Settings → Applications →
   Installed GitHub Apps**.

The App's permissions and events are already configured (Repository: Checks,
Contents, Issues and Pull requests `Read & Write`, Metadata `Read-only`;
Organization: Members `Read-only`; events `check_run`, `check_suite`,
`commit_comment`, `issue_comment`, `pull_request`, `push`). Webhooks arrive via
the gosmee relay (`hook.pipelinesascode.com`); no public IP or domain is
needed. Do not create a per-owner App or a second controller.

### 4. Create the tier secrets out-of-band

Secrets are never committed. The namespace is the secret boundary: any Secret
in a tenant namespace is readable by that tenant's PipelineRuns, because secret
mounts bypass RBAC.

**Build tenant — tenant-supplied registry credentials.** Create a
`docker-registry` Secret in the tenant namespace and reference it from the
tenant's `.tekton/` `docker-credentials` workspace:

```sh
kubectl -n <tenant-namespace> create secret docker-registry <tenant-registry-secret> \
  --docker-server=<registry-host> \
  --docker-username=<registry-user> \
  --docker-password=<registry-token> \
  --docker-email=<registry-email>
```

The tenant owns this credential. Never reuse the operator's Docker Hub, GHCR,
AWS or GitOps credentials, and never place them in a `build` namespace.

**Deploy tenant — operator deploy secrets.** The operator-controlled deploy
tenant gets the credentials its GitOps path needs. These are rendered or
created out-of-band in the deploy tenant namespace, never in a build namespace:

| Object | Created by | Purpose |
|---|---|---|
| `dockerhub-docker-config` | shared `docker-build-publish-secrets` chart (real file gitignored) | image push |
| `home-terraform-plans` PVC + cleanup CronJob | shared `terragrunt-secrets` chart | plan/apply handoff (RWO; one pair at a time) |
| `gitops-pat` | operator, out-of-band; `*.example.yaml` only in Git | contents:write on `home-cluster` for the GitOps bump |
| `cloud-creds` | operator, out-of-band; example only | cloud API access for deploy tasks |
| `npm-secret` | operator, out-of-band; optional | private npm packages, if used |

```sh
kubectl -n <deploy-tenant-namespace> create secret generic gitops-pat \
  --from-literal=username=tekton-bot \
  --from-literal=password='<GITHUB_PAT>'

kubectl -n <deploy-tenant-namespace> create secret generic cloud-creds \
  --from-literal=CLOUD_ACCESS_KEY_ID='<ACCESS_KEY_ID>' \
  --from-literal=CLOUD_SECRET_ACCESS_KEY='<SECRET_ACCESS_KEY>'
```

`gitops-pat` and `cloud-creds` do not exist until the operator creates them.
The committed files are examples only. A build tenant never receives them.

### 5. Developer visibility (optional)

`developers:` produces:

- one `RoleBinding tekton-developer-<email-slug>` in the tenant namespace,
  binding `User: <email>` to the read-only `tekton-developer` ClusterRole;
- one deduped `ClusterRoleBinding tekton-namespace-lister-<email-slug>` per
  unique email, for the dashboard namespace dropdown.

The email is the developer's **Cognito** identity (the address they sign in
with), not necessarily their GitHub address; the proxy injects it as
`Impersonate-User` and the binding is `User: <email>`.

The developer can `get/list/watch` PipelineRuns, TaskRuns, Pipelines, Tasks,
CustomRuns, Repository CRs, pods and events, and `get` pod logs, in their tenant
namespace. They cannot see Secrets or ConfigMaps, cannot create/update/delete,
cannot exec/attach/port-forward, and cannot see resources in another tenant
namespace. Admin read/write stays cluster-wide and is never granted to a tenant
user.

Verify after the apply:

```sh
kubectl auth can-i get pipelineruns -n <tenant-namespace> --as=<developer-email>  # yes
kubectl auth can-i get pods/log     -n <tenant-namespace> --as=<developer-email>  # yes
kubectl auth can-i get secrets      -n <tenant-namespace> --as=<developer-email>  # no
kubectl auth can-i create pipelineruns -n <tenant-namespace> --as=<developer-email> # no
kubectl auth can-i get pods -n <other-tenant-namespace> --as=<developer-email>    # no
kubectl auth can-i list namespaces --as=<developer-email>                         # yes
```

The dashboard runs with `--namespaces=` (empty) and impersonates the Cognito
email via oauth2-proxy, so the API evaluates the viewer's own RBAC. Do not
re-add a namespace pin. The live acceptance matrix is
`home-sso-auth-rbac/tests/developer-rbac-checks.sh`.

### 6. Verify the tenant

```sh
kubectl get ns <tenant-namespace> --show-labels
kubectl get resourcequota,limitrange,ciliumnetworkpolicy,serviceaccount,configmap \
  -n <tenant-namespace>
kubectl get repositories.pipelinesascode.tekton.dev -n <tenant-namespace>

# Repository URL and provenance
kubectl get repositories.pipelinesascode.tekton.dev -n <tenant-namespace> \
  -o custom-columns=NAME:.metadata.name,URL:.spec.url,PROVENANCE:.spec.settings.pipelinerun_provenance

# No operator secrets in a build tenant (only PaC per-run git auth Secrets, if any)
kubectl get secrets -n <tenant-namespace>
```

Then push a commit or open a pull request in a registered repository. The PaC
check run should start a PipelineRun **in the tenant namespace**:

```sh
kubectl get pipelinerun -n <tenant-namespace> -w
kubectl logs -n <tenant-namespace> -l tekton.dev/pipelineRun=<run-name> --all-containers --prefix
```

Adding a repository to an existing tenant is only a registry edit + apply. The
chart creates the new Repository CR; no new namespace, App or controller is
needed.

## Remove or offboard a tenant

> **Data loss warning.** Deleting a tenant namespace deletes its PVCs. Longhorn
> volumes with the default reclaim policy `Delete` are removed with the PVC, so
> workspace and plan data is **unrecoverable**. Back up anything that must
> survive, or set the PV reclaim policy to `Retain` before applying. There is no
> undo.

Do this in order:

1. **Uninstall the App from the owner.** GitHub → the owner's **Settings →
   Applications → Installed GitHub Apps → Configure → Uninstall**. This stops
   webhooks and revokes installation tokens immediately. If the owner is an
   organization, an org owner performs it.
2. **Remove the registry entry** from `values/tenants.yaml` and apply the same
   command as above. A **post-install/post-upgrade hook** (the pruner):
   - lists namespaces labelled `home.cluster/managed=true`;
   - deletes those whose name is not in the registry
     (`kubectl delete namespace --wait=false`);
   - deletes `tekton-namespace-lister-*` ClusterRoleBindings whose subject is no
     longer in the deduped developer set.
3. **Confirm the namespace is gone:**

   ```sh
   kubectl get ns -l home.cluster/managed=true
   kubectl get repositories.pipelinesascode.tekton.dev -A
   kubectl get clusterrolebinding | grep tekton-namespace-lister
   ```

### What the pruner deletes — and what it does not

| Object | Deleted by the pruner? | Notes |
|---|---|---|
| Tenant `Namespace` | Yes, if labelled `home.cluster/managed=true` | Deletion is asynchronous (`--wait=false`) |
| Repository CRs, PipelineRuns, TaskRuns, pods, PVCs, Secrets, ConfigMaps, RoleBindings in the namespace | Yes, with the namespace | Longhorn volumes follow the PVC reclaim policy |
| Stale `tekton-namespace-lister-*` CRBs | Yes | Only if no remaining tenant lists that developer |
| The GitHub App installation | **No** | Uninstall manually (step 1) |
| The global `ValidatingAdmissionPolicy` | **No** | Cluster-wide; unrelated to one tenant |
| Repository CRs in another namespace (e.g. an old `tekton-pipelines` CR) | **No** | Delete manually; see below |
| The App private key in `pipelines-as-code` | **No** | Shared by all installations; rotate only if compromised |

### Secret cleanup

- Secrets in the tenant namespace are deleted with the namespace. If you keep
  the namespace for a rebuild, delete the tenant's registry Secret and any
  deploy secrets explicitly.
- `dockerhub-docker-config`, `gitops-pat`, `cloud-creds` and `npm-secret` in a
  deploy tenant namespace disappear with that namespace. Remove any copies from
  password managers or local files; the App private key is unaffected.
- The single App key can mint tokens for every installation. Uninstalling the
  App is what revokes that tenant's access. If the key itself leaked, rotate it
  in the App settings and update the `pipelines-as-code-secret` Secret; that
  affects every tenant.

### Old Repository CRs

The pre-tenancy Repository CRs lived in `tekton-pipelines`. The admission gate
now denies a Repository CR in a namespace without a `home.cluster/tenant`
label, so they cannot be recreated there. If any remain, delete them after
confirming no run needs them:

```sh
kubectl get repositories.pipelinesascode.tekton.dev -A \
  -o custom-columns=NS:.metadata.namespace,NAME:.metadata.name,URL:.spec.url
kubectl delete repository.pipelinesascode.tekton.dev <name> -n <namespace>
```

PaC enforces globally unique Repository URLs. A stale CR for the same URL in
another namespace makes the webhook ambiguous and is a common cause of "no run"
or duplicate-run symptoms.

## Tier capabilities

| Capability | `build` | `deploy` |
|---|---|---|
| Kubernetes API egress from pods | **denied** | **allowed** (`toEntities: kube-apiserver`) |
| Home LAN, node IPs, RFC1918, link-local | denied | denied |
| Cluster DNS, Verdaccio `:4873`, public internet | allowed | allowed |
| Ingress | host only (kubelet probes) | host only (kubelet probes) |
| Registry credentials | tenant-supplied, out of band | operator-supplied |
| Deploy credentials | none | as required by the GitOps path |
| Pod Security | `restricted` | `restricted` |
| Developer role | read-only | read-only |
| Intended owner | external tenant | owner the cluster operator controls |

### Quota and LimitRange defaults

Per-tenant `quota:` and `limitRange:` maps in the registry override the tier
defaults from `charts/tenant-platform/values.yaml`.

| Resource | `build` | `deploy` |
|---|---|---|
| `requests.cpu` / `limits.cpu` | 4 / 8 | 8 / 16 |
| `requests.memory` / `limits.memory` | 8Gi / 16Gi | 16Gi / 32Gi |
| `requests.ephemeral-storage` / `limits.ephemeral-storage` | 10Gi / 20Gi | 20Gi / 40Gi |
| `pods` | 20 | 40 |
| `persistentvolumeclaims` | 4 | 8 |
| `requests.storage` | 40Gi | 100Gi |
| LimitRange default cpu / memory / ephemeral | 2 / 2Gi / 5Gi | 4 / 4Gi / 10Gi |
| LimitRange defaultRequest cpu / memory / ephemeral | 200m / 256Mi / 1Gi | 250m / 512Mi / 1Gi |

A quota is a hard bound on resources, not on outbound traffic. A `build` tenant
can still reach the public internet (git, package downloads, registry pushes);
that is required and is recorded as a residual risk.

### Rootless builder limitations

`build-push-v1` is rootless Buildah in a single-mapping user namespace (the
only mapping `restricted` Pod Security allows with `allowPrivilegeEscalation:
false`, which sets `no_new_privs` and blocks `newuidmap`). Consequences:

- **Builds work** for `FROM scratch` and for images whose files are all owned
  by UID/GID 0 (for example a statically linked Go binary copied into a scratch
  image). Verified live in `stijnklomp-lave` under `restricted`: a
  `FROM scratch` build pushed to an anonymous registry.
- **Builds fail** for base images that contain files owned by another UID/GID,
  including `alpine`, `nginx:*` and most distribution images: `/etc/shadow` is
  owned by `root:shadow` (`0:42`). The error is
  `potentially insufficient UIDs or GIDs available in user namespace
  (requested 0:42 for /etc/shadow)`. The same limitation applies to rootless
  BuildKit (it needs `newuidmap`) and to `hostUsers: false` pods unless the pod
  runs as UID 0 inside the user namespace.

Do **not** relax the namespace's `restricted` enforcement or add capabilities
to work around it. The supported options are:

1. Build from `FROM scratch` / distroless-style images whose files are all
   `0:0`, or pre-flatten the base image to `0:0` in a trusted rootful job and
   push it to a registry the tenant can pull from.
2. Change the platform: run tenant build pods with `hostUsers: false` (a full
   65536 UID/GID mapping) as UID 0 *inside the user namespace*, and replace the
   `restricted` namespace label with a user-namespace-aware admission policy
   that still forbids host namespaces, hostPath, privileged, added capabilities
   and privilege escalation. That is a design change with its own ADR, not a
   registry tweak.

The limitation is about **image ownership metadata**, not about network or
secret isolation: the build still runs in the tenant namespace, under the tenant
network policy and quota, with no operator credentials.

## Secrets model

| Object | Owner / creator | Namespace | Notes |
|---|---|---|---|
| Per-run git auth Secret | PaC | tenant namespace | Created per run, scoped to the triggering repository; name exposed as `{{ git_auth_secret }}` |
| Tenant registry credentials | Tenant (operator creates out-of-band) | build tenant namespace | Tenant-supplied; never the operator's credentials |
| `dockerhub-docker-config` | shared `docker-build-publish-secrets` chart | deploy tenant namespace | Operator image-push credential |
| `home-terraform-plans` PVC + cleanup CronJob | shared `terragrunt-secrets` chart | deploy tenant namespace | RWO volume; one plan+apply pair at a time |
| `gitops-pat` | Operator, out-of-band | deploy tenant namespace | Example only in Git; contents:write on `home-cluster` |
| `cloud-creds` | Operator, out-of-band | deploy tenant namespace | Example only in Git |
| `npm-secret` | Operator, out-of-band (optional) | deploy tenant namespace | Only for private packages |
| `npm-npmrc` ConfigMap | `tenant-platform` | every tenant namespace | Non-secret; Verdaccio endpoint for `npm-install-v1` |

Rules:

- **The namespace is the boundary.** Secret mounts bypass RBAC: any Secret in a
  namespace is readable by that tenant's PipelineRuns. RBAC on Secrets protects
  the dashboard view, not the workload.
- A `build` namespace must contain no operator credentials (Docker Hub, GHCR,
  AWS, GitOps). A `deploy` namespace holds the credentials its GitOps path
  needs.
- A tenant's `tekton-bot` ServiceAccount has no RoleBinding. It cannot read
  Secrets through the API; it can still mount a Secret that the PipelineRun
  references in its own namespace.
- Cross-tenant reads are impossible by namespace scoping. Verify:

  ```sh
  kubectl auth can-i get secrets -n <deploy-tenant-namespace> \
    --as=system:serviceaccount:<build-tenant-namespace>:tekton-bot   # no
  ```

## Developer visibility

| Piece | Scope | What it grants |
|---|---|---|
| `tekton-developer` ClusterRole | cluster (owned by the SSO/RBAC repo) | `get/list/watch` on `pipelineruns`, `taskruns`, `pipelines`, `tasks`, `customruns`, `repositories`, `pods`, `events`; `get` on `pods/log`; no Secrets, ConfigMaps, writes, exec, attach or port-forward |
| `tekton-developer-<email-slug>` RoleBinding | one per developer per tenant namespace | Binds `User: <email>` to `tekton-developer` |
| `tekton-namespace-lister` ClusterRole | cluster (owned by the SSO/RBAC repo) | `get/list` Namespaces (dashboard dropdown only) |
| `tekton-namespace-lister-<email-slug>` ClusterRoleBinding | one per unique email, deduped across tenants | Binds the developer to the lister role |

- The dashboard runs with `--namespaces=` empty. oauth2-proxy injects
  `Impersonate-User: <Cognito email>`; the API evaluates the viewer's RBAC.
  Admins keep cluster-wide `admin-read`/`admin-write`; tenant users never get
  admin.
- A developer may see namespace **names** but not resources in namespaces where
  they have no RoleBinding.
- Viewer identity and workload identity are independent: a developer's
  read-only Role does not limit what their pipeline can mount, and a user who
  can create a PipelineRun in a namespace can use that namespace's workload
  identity. That is why the developer role is read-only.

## Network policy

Cilium is required. On Flannel the `CiliumNetworkPolicy` is accepted and
ignored. The chart creates one `tenant-sandbox` policy per tenant namespace:

```yaml
apiVersion: cilium.io/v2
kind: CiliumNetworkPolicy
metadata:
  name: tenant-sandbox
  namespace: <tenant-namespace>
spec:
  endpointSelector: {}
  ingress:
    - fromEntities: [host]          # kubelet probes / host access only
  egress:
    - toEndpoints:
        - matchLabels:
            "k8s:io.kubernetes.pod.namespace": kube-system
            "k8s:k8s-app": kube-dns
      toPorts:
        - ports:
            - {port: "53", protocol: UDP}
            - {port: "53", protocol: TCP}
    - toEndpoints:
        - matchLabels:
            "k8s:io.kubernetes.pod.namespace": verdaccio
      toPorts:
        - ports:
            - {port: "4873", protocol: TCP}
    - toEntities: [world]           # public internet
    # deploy only:
    - toEntities: [kube-apiserver]
  egressDeny:                       # takes precedence over the world allow
    - toCIDR: [192.168.0.0/16]      # home LAN and node IPs
    - toCIDR: [172.16.0.0/12]
    - toCIDR: [169.254.0.0/16]      # link-local and metadata
```

The pod and service CIDRs are intentionally not denied: that would also deny
DNS and Verdaccio. Other namespaces and the API server are ClusterIPs and are
denied by default because they are not in the allow list. `deploy` adds only
`toEntities: [kube-apiserver]`; it does not add LAN or peer-namespace access.

### Verify the policy

| Check | Command | Expected |
|---|---|---|
| Cilium is the CNI | `kubectl -n kube-system exec ds/cilium -- cilium-dbg status --brief` | `OK` |
| Policy is present | `kubectl get cnp -n <tenant-namespace>` | `tenant-sandbox` |
| Policy shape | `kubectl describe cnp tenant-sandbox -n <tenant-namespace>` | egress rules as above; deploy has the API rule |
| Tenant isolation | `home-cilium/verify-isolation.sh` from the workspace root | `RESULT: PASS - tenant isolation holds` |
| Build tenant probe | apply `tenant-platform/tests/build-tenant-pipelinerun.yaml`; wait for `Succeeded` | DNS/Verdaccio/internet PASS; API, LAN, peer denied |
| Deploy tenant probe | `KUBECONFIG_DIR=/path/to/home-cluster-1-config ./tenant-platform/tests/deploy-tenant-checks.sh` from the cicd repo root | `PASSED: all deploy-tenant checks`; API reachable (HTTP 401), DNS/Verdaccio/internet PASS |
| Admission gate | `KUBECONFIG_DIR=/path/to/home-cluster-1-config ./tenant-platform/tests/admission-checks.sh` | matching owner admitted; mismatched/missing-label denied; exemption admitted |
| Clean rebuild (destructive) | `KUBECONFIG_DIR=/path/to/home-cluster-1-config ./tenant-platform/tests/clean-rebuild-checks.sh` | deletes and rebuilds the **build** tenant from Git; `PASSED: clean rebuild recreated the build tenant and the acceptance run passed` |
| Offline render | `./tenant-platform/tests/render-checks.sh` | all checks pass; no cluster access |

`verify-isolation.sh` creates the scratch namespaces `t2-verify` and
`t2-verify-peer`, applies a copy of the tenant policy, starts a peer and runs
one Job. It deletes both namespaces on exit; `--keep` keeps them for debugging.
It is expected to fail on Flannel by design.

The clean-rebuild check is **destructive**: it deletes the build tenant
namespace (and any PVCs in it) before rebuilding from Git. It refuses to touch
the deploy tenant and runs the control-plane preflight first; do not adapt it to
a tenant that holds data you need.

### `cilium-dbg` and Hubble

Hubble is **disabled** in `home-cilium/values/cilium-values.yaml`, so there is
no flow UI or `hubble observe` today. Use the agent:

```sh
kubectl -n kube-system exec ds/cilium -- cilium-dbg status --brief
kubectl -n kube-system exec ds/cilium -- cilium-dbg endpoint list
kubectl -n kube-system exec ds/cilium -- cilium-dbg monitor --type drop
```

`cilium-dbg monitor` streams live verdicts; run it while reproducing a denied
connection. If Hubble is enabled later, `hubble observe --namespace
<tenant-namespace> --verdict DROPPED` gives the same information with historical
flows.

## Troubleshooting

| Symptom | Likely cause | Check / fix |
|---|---|---|
| Repository CR denied by the admission gate | URL owner differs from the namespace's `home.cluster/tenant` label, the label is missing, or the CR is in `tekton-pipelines` | Read the denial: it names the expected URL. Check `kubectl get ns <ns> -o jsonpath='{.metadata.labels.home\.cluster/tenant}'`; fix the registry entry (owner/URL/namespace) and re-apply. Only `pipelines-as-code` is exempt. Do not weaken or remove the policy. |
| Webhook rejected or ambiguous; no run starts | Two Repository CRs match the same URL; PaC enforces globally unique URLs | `kubectl get repositories.pipelinesascode.tekton.dev -A -o custom-columns=NS:.metadata.namespace,NAME:.metadata.name,URL:.spec.url`; delete the stale CR (usually the old `tekton-pipelines` one) after confirming no run needs it; re-apply the registry. During a cutover, delete the old CR **before** applying the new one. |
| Pod rejected with `violates PodSecurity "restricted:latest"` | Task image runs as root, lacks a seccomp profile, adds capabilities, or Tekton's injected containers have no security context | `kubectl get events -n <ns> --sort-by=.lastTimestamp`; `kubectl describe taskrun -n <ns> <taskrun>` and `kubectl describe pod -n <ns> <pod>`. Make the step non-root (UID 65532; build step UID 1000), `allowPrivilegeEscalation: false`, `capabilities.drop: [ALL]`, `seccompProfile: RuntimeDefault` (build step: Localhost `profiles/buildah-userns.json`), and `fsGroup: 65532`. Confirm TektonConfig `set-security-context: true`. |
| Build fails at `unshare(CLONE_NEWUSER)` | Node lacks `user.max_user_namespaces=15000`, the `buildah-userns` seccomp profile, or both | Check every node: `talosctl read /proc/sys/user/max_user_namespaces -n <node>` and `talosctl ls /var/lib/kubelet/seccomp/profiles -n <node>`. Apply `patches/user-namespaces.yaml` (immediate) and `patches/buildah-seccomp.yaml` (`--mode=staged`, then reboot). A node that has not rebooted still needs the profile installed out-of-band. |
| Build fails with `potentially insufficient UIDs or GIDs ... requested 0:42 for /etc/shadow` | The base image has files owned by a UID/GID outside the single mapping rootless Buildah gets under `restricted` (common: `alpine`, `nginx`, most distro images) | This is the rootless limitation described in **Rootless builder limitations**. Build from `FROM scratch` / an all-`0:0` image, or pre-flatten the base image. Do not relax PSA or add capabilities. |
| `more than one PersistentVolumeClaim is bound` / PVC pending | Tekton coschedule is enabled, or two RWO Longhorn volumes are requested in one TaskRun | Confirm TektonConfig `coschedule: disabled`. Use one plan+apply pair at a time per deploy tenant (the plan PVC is RWO). Use `emptyDir` for probes that do not need persistence. `kubectl get pvc -n <ns>`; `kubectl describe pvc -n <ns> <pvc>`. |
| Namespace stuck in `Terminating` | Finalizers, pods that will not stop, PVC/Longhorn volume cleanup, or a failing webhook | `kubectl get ns <ns> -o json`; `kubectl get all,pvc,repositories.pipelinesascode.tekton.dev -n <ns>`; `kubectl -n longhorn-system get volumes.longhorn.io`. Wait for Longhorn to detach; remove a finalizer only after confirming the resource is gone and the data is backed up. The pruner uses `--wait=false`, so a terminating namespace is normal for a while. |
| PaC does not trigger | Repository not in the registry, App not installed on the owner, auto-configure off, webhook relay down, missing/wrong `.tekton/` annotations, duplicate URL, or `/ok-to-test` required | `kubectl get repositories.pipelinesascode.tekton.dev -n <tenant-namespace>`; GitHub App → **Advanced → Recent Deliveries**; `kubectl -n pipelines-as-code logs deploy/pipelines-as-code-controller`; `kubectl -n pipelines-as-code logs deploy/gosmee-client`; confirm the repo has `.tekton/*.yaml` on the event branch with `on-event`/`on-target-branch`. Unknown installations are inert by design. |
| Developer sees no tenant resources in the dashboard | Email mismatch, binding not applied, stale dashboard namespace pin, or missing namespace-lister CRB | `kubectl get rolebinding -n <tenant-namespace> \| grep tekton-developer`; `kubectl get clusterrolebinding \| grep tekton-namespace-lister`; `kubectl auth can-i ... --as=<email>`; confirm the dashboard runs with `--namespaces=` and impersonation. Re-apply the registry after editing `developers`. |
| Image push fails with 401/403 | Tenant registry Secret missing or referenced by the wrong name; `.tekton/` uses the operator's secret name | `kubectl get secrets -n <tenant-namespace>`; create the tenant's `docker-registry` Secret; point the `docker-credentials` workspace at it. A build tenant must not use `dockerhub-docker-config`. |

## Day-2 changes

| Change | Action | Notes |
|---|---|---|
| Add a repository | Add a `repos[]` entry, apply | Only a registry edit; no new namespace/App |
| Remove a repository | Remove the `repos[]` entry, apply | Helm removes the Repository CR; running PipelineRuns continue |
| Add/remove a developer | Edit `developers`, apply | RoleBindings follow; stale namespace-lister CRBs are pruned |
| Change a tier | Edit `tier`, apply | `build → deploy` adds API egress but **not** deploy secrets; `deploy → build` requires deleting the deploy secrets and the plan PVC/CronJob from the namespace first |
| Rotate the App key | Update `pipelines-as-code-secret`, restart the controller | Affects every installation; the App is shared |
| Transfer App ownership | Transfer the App to the operator organization | Recommended; a personal-account App dies with the account |
| Change a repository URL | Update `repos[].url`, apply | PaC URL uniqueness: delete the old CR first if the URL moves between tenants |

## Rollback

### Tenancy rollback (normal)

1. Revert the registry change in Git and apply the tenant platform again. The
   pruner deletes managed namespaces that are no longer in the registry and
   stale namespace-lister CRBs. Re-adding a removed tenant recreates empty
   objects; it does not restore PVC data, and the App installation must be
   created again.
2. If the chart itself regressed, `helm history tenant-platform` and
   `helm rollback tenant-platform <revision>` (Helm stores the release in the
   kubeconfig default namespace). A chart rollback does not necessarily delete
   objects that were removed from the templates; the registry + pruner is the
   reliable path for tenant removal.
3. Back up PVC data before removing a tenant; Longhorn volumes follow the PVC
   reclaim policy and are not recoverable.

### Restore the old single-namespace Repository CRs

Only for a full rollback to the pre-tenancy model. This reverses the admission
gate, the shared-chart namespace move and the registry.

1. Freeze CI. Do not run old and new CRs for the same URL at the same time.
2. Delete the admission gate, binding first:

   ```sh
   kubectl delete validatingadmissionpolicybinding tenant-platform-repository-owner
   kubectl delete validatingadmissionpolicy tenant-platform-repository-owner
   ```

   Without this, a Repository CR in `tekton-pipelines` (no tenant label) is
   denied by design.
3. Restore the pre-tenancy `pipelines-as-code-repositories` chart and its
   Repository CRs from Git history (the chart was deleted by the tenancy
   cleanup; use the commit before the cleanup, or `main` if it still exists),
   then apply it. Do not hand-create CRs that the chart should own.
4. Move the shared deploy objects back to `tekton-pipelines`: set
   `namespace: tekton-pipelines` in
   `developer/pipelines/values/platform.yaml` and re-apply the shared
   charts. Recreate `dockerhub-docker-config` from its gitignored file in the
   new namespace. The plan PVC data was ephemeral and was not copied; treat it
   as lost.
5. Re-enable PaC auto-configure only if you actually want it; it is a separate
   ConfigMap change and is not required for the rollback.
6. After the old CRs run successfully, remove the tenant namespaces (registry
   removal + apply, or `kubectl delete namespace`). Back up first.
7. To roll back only the admission gate, delete the binding and policy (step 2)
   or revert the tenant chart. To roll back only one tenant, remove its registry
   entry; do not delete the cluster-wide gate.

### Cilium rollback

The CNI rollback is cluster-wide and disruptive: pod networking is unavailable
between Cilium teardown and Flannel restoration. The full procedure (freeze CI
and ArgoCD, `cni.uninstall=true`, `helmfile destroy`, restore the Flannel patch
on every node, reboot one at a time, restart workloads, optional CRD deletion)
is in
[`home-cluster-kubernetes-base/home-cilium/README.md`](https://github.com/stijnklomp-talos-linux-home-setup/home-cluster/blob/main/platform/cluster/home-cilium/README.md#rollback-to-flannel).
Do not duplicate it here; follow it in one maintenance window.

Tenancy-specific warnings for that rollback:

- Deleting the Cilium CRDs deletes every tenant `CiliumNetworkPolicy`. Do not
  run tenant workloads during the rollback window.
- Flannel ignores the policies, so a tenant namespace left running on Flannel
  has no network isolation. Finish the rollback or stop the tenants.
- The tenant chart depends on the `CiliumNetworkPolicy` CRD; apply it only
  after Cilium (and its CRD) is back.

### Talos user-namespace and seccomp prerequisites

These are not part of the tenancy rollback, but a rebuilt cluster or a new node
must have them before rootless builds work:

| Patch | Purpose | Apply |
|---|---|---|
| `platform/node/patches/user-namespaces.yaml` | `user.max_user_namespaces: "15000"` (Talos default 0) | From `platform/node/`: `talosctl patch machineconfig --nodes <node> --patch @patches/user-namespaces.yaml` (immediate; persists at boot) |
| `platform/node/patches/buildah-seccomp.yaml` | installs `/var/lib/kubelet/seccomp/profiles/buildah-userns.json` | From `platform/node/`: `talosctl patch machineconfig --nodes <node> --mode=staged --patch @patches/buildah-seccomp.yaml`, then reboot |

Apply to every node that can run a tenant PipelineRun. Verify with
`talosctl read /proc/sys/user/max_user_namespaces -n <node>` (expect `15000`) and
`talosctl ls /var/lib/kubelet/seccomp/profiles -n <node>` (expect
`buildah-userns.json`). Without the profile, builds fail at
`unshare(CLONE_NEWUSER): operation not permitted`; without the sysctl they fail
with `no space left on device`.

## Verified live (2026-10-24)

Cluster: `home-cluster-1`, Talos `v1.13.9`, Kubernetes `v1.36.4`, 4 nodes.
Cilium `1.20.4`; PaC `v0.50.0`; Tekton Pipelines v1.15 (operator 0.81.1).
Evidence below is from the live acceptance runs; re-run the scripts after any
change.

| Area | Evidence |
|---|---|
| Cilium | `cilium-dbg status --brief` → `OK`; `verify-isolation.sh` → `RESULT: PASS - tenant isolation holds`; allowed DNS, Verdaccio `:4873`, `registry.npmjs.org`; denied the API service, node API, Talos API, node `:443`, ingress LoadBalancer, `169.254.169.254` and a peer namespace |
| Rootless build | `build-push-v1` (quay.io/buildah/stable:v1.41.5, chroot, vfs) built and pushed `ttl.sh/pac-tenancy-rootless-ok2:1h` (digest `sha256:5f5ff65b…`); `app-release` end-to-end 3/3 tasks succeeded (`ttl.sh/pac-tenancy-app-full2:1h`); no root task remains |
| Build tenant | `stijnklomp-lave` created with `home.cluster/tenant=stijnklomp-LAVE`, `tier=build`, `restricted` PSA; quota, LimitRange, `tenant-sandbox` CNP, `tekton-bot` (no automount), `npm-npmrc`, `test-app` Repository CR (`concurrency_limit: 2`, `source` provenance); `tenant-acceptance` PipelineRun **Succeeded**: DNS/Verdaccio/internet PASS, API/home LAN/peer namespace denied; no Secrets in the namespace; PSA events clean |
| Prune test | Temporary registry entry `scratch` / `scratch-tenant` applied → namespace created; entry removed + apply → post-upgrade pruner deleted the namespace |
| Deploy tenant | Org namespace `stijnklomp-talos-linux-home-setup` with deploy quota (8 CPU / 16Gi, 40 pods, 100Gi), `restricted` PSA and a CNP with `toEntities: [kube-apiserver]`; 17 org Repository CRs, 1 build CR, 0 in `tekton-pipelines`; `dockerhub-docker-config` only in the org namespace; build tenant has no Secrets; `auth can-i get secrets` as the build `tekton-bot` in the org namespace = **no**; `deploy-tenant-acceptance` PipelineRun **Succeeded**: API HTTP 401 (reachable), DNS/Verdaccio/internet PASS; build acceptance re-run: API still denied |
| Developer RBAC | Live matrix with a synthetic `test-dev@example.com`: allowed `get/list/watch` on PipelineRuns, TaskRuns, Pipelines, Tasks, CustomRuns, Repository CRs, pods and events, `get` on pod logs, and namespace list; denied Secrets, ConfigMaps, all writes, exec/attach/port-forward and every resource in another namespace; admin unchanged. Dashboard through impersonation: namespaces list includes the tenant, tenant resources 200, Secrets/ConfigMaps 403, other namespace 403 |
| Admission gate | `tenant-platform-repository-owner` + binding (`failurePolicy: Fail`, `Deny`, CREATE/UPDATE, `pipelines-as-code` exempt); live: matching owner admitted, mismatched owner denied, namespace without the label denied, UPDATE changing the owner denied, exemption namespace admitted, existing `stijnklomp-lave/test-app` intact |
| Clean rebuild | **Passed** with ticket #8's `tenant-platform/tests/clean-rebuild-checks.sh`: deleted `stijnklomp-lave`, rebuilt it from Git with `helmfile -l stage=tenancy sync`, and asserted every registry-managed object returned (namespace labels/PSA, quota, LimitRange, CNP, `tekton-bot`, `npm-npmrc`, Repository CR, developer bindings); `tenant-acceptance` **Succeeded** and the control plane has 0 Repository CRs, no `tekton-bot` and no `npm-npmrc` |

Acceptance lines:

```text
verify-isolation.sh: RESULT: PASS - tenant isolation holds
build tenant:       tenant-acceptance Succeeded — DNS/Verdaccio/internet PASS; API/home LAN/peer denied
deploy tenant:      deploy-tenant-acceptance Succeeded — API HTTP 401 (reachable); DNS/Verdaccio/internet PASS
deploy checks:      PASSED: all deploy-tenant checks
clean rebuild:      PASSED: clean rebuild recreated the build tenant and the acceptance run passed
retire checks:      PASSED: single-namespace path is retired; registry tenants intact
```

Residual: the GitHub webhook round-trip (App installation → PaC → check run)
cannot be exercised from the implementation environment. The PipelineRuns above
simulate the PaC-created run; the webhook path is verified by pushing to a
registered repository after installing the App.

## Related docs

| Doc | What it covers |
|---|---|
| [spec.md](./spec.md) | Contract: registry schema, per-tenant objects, network policy, RBAC, admission, tiers |
| [pac-tenancy-adr.md](../../../architecture-decision-log/home/pac-tenancy-adr.md) | Decision: one run namespace per GitHub owner |
| [cilium-networkpolicy-adr.md](../../../architecture-decision-log/home/cilium-networkpolicy-adr.md) | Decision: Cilium for tenant network isolation |
| [../setup/README.md](../setup/README.md) | How a repository becomes PaC-enabled |
| [../tips/README.md](../tips/README.md) | Day-to-day `.tekton/` tips |
| [../troubleshoot/README.md](../troubleshoot/README.md) | General pipeline troubleshooting |
| [tenant-platform README](https://github.com/stijnklomp-talos-linux-home-setup/home-cluster/blob/main/platform/cicd/tenant-platform/README.md) | Chart layout, registry validation, live checks |
| [home-cilium README](https://github.com/stijnklomp-talos-linux-home-setup/home-cluster/blob/main/platform/cluster/home-cilium/README.md) | Cilium rollout and rollback |
| [Talos (platform/node) README](https://github.com/stijnklomp-talos-linux-home-setup/home-cluster/blob/main/platform/node/README.md) | Talos patches and node prerequisites |
