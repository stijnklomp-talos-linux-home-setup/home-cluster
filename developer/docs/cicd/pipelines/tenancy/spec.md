# PaC tenancy — specification

## Status

Accepted (2026-10); implemented — see [Implementation status](#implementation-status-2026-10-24). Accepted decisions: [pac-tenancy-adr.md](../../../architecture-decision-log/home/pac-tenancy-adr.md) and [cilium-networkpolicy-adr.md](../../../architecture-decision-log/home/cilium-networkpolicy-adr.md). Vocabulary: [GLOSSARY.md](../../../GLOSSARY.md).

This document specifies *what* the system must do. It does not prescribe the implementation.

## Implementation status (2026-10-24)

Implemented and verified live on `home-cluster-1` (Talos `v1.13.9`, Kubernetes
`v1.36.4`): Cilium tenant isolation, `restricted` rootless builds, the build and
deploy tenants, read-only developer RBAC, the Repository admission gate and
auto-configure off. Operator procedures are in [runbook.md](./runbook.md); the
evidence summary is in
[runbook.md#verified-live-2026-10-24](./runbook.md#verified-live-2026-10-24).

Residual: the GitHub App webhook round-trip was not exercised from the
implementation environment. The clean rebuild was verified live on 2026-10-24
with ticket #8's `tenant-platform/tests/clean-rebuild-checks.sh`. The decision
text below is unchanged.

## Purpose

Define how Pipelines-as-Code (PaC) serves multiple GitHub owners (personal accounts and organizations) from one cluster without sharing secrets, credentials or network reachability between them.

## Goals

- Add or remove a GitHub owner by editing one registry file and installing the public GitHub App.
- Use one public GitHub App and one PaC controller. No per-owner App, private key or webhook endpoint.
- Run every repository's PipelineRuns in its owner's namespace.
- Prevent a tenant from reading another tenant's secrets, PipelineRuns or logs through Kubernetes.
- Prevent a tenant from reaching another tenant, the home LAN, the Talos API or (for `build`) the Kubernetes API server over the network.
- Let tenants change `.tekton/` in feature branches, exactly as on the default branch.
- Give tenant developers read-only visibility of their own namespace; keep cluster administration separate.
- Keep Git the source of truth: no Repository CR or namespace exists only in the cluster.

## Non-goals

- Per-repository namespaces. The tenant unit is the GitHub owner.
- Per-owner GitHub Apps and controller shards. Deferred until a tenant requires its own key.
- Kernel or node isolation on the shared CI node.
- Open registration. The App is public, but the registry decides whose code runs.
- A management UI. The registry is the management interface.
- Running untrusted code without PSA, quota and network policy.

## Model

- **Tenant**: a GitHub owner admitted to CI.
- **Tenant namespace**: the namespace allocated to one tenant; it is also the run namespace.
- **Registry**: the single declarative list of tenants, namespaces, tiers, repositories and developer emails.
- **Run namespace**: the namespace of the Repository CR that matched the webhook; PaC creates the PipelineRun there.
- **Repository CR**: the PaC `Repository` object that registers one git repository.

The registry is the complete list. PaC auto-configure is disabled, so an unlisted repository never runs and an unknown installation is inert.

## Registry

The registry is a YAML file with this schema:

```yaml
tenants:
  - owner: <github-owner>          # required, unique
    namespace: <dns-1123-label>    # required, unique; run namespace
    tier: build | deploy           # required
    developers:                    # optional
      - <developer-email>
    repos:                         # required, may be empty
      - name: <dns-subdomain>      # required, unique per namespace
        url: <https://github.com/owner/repo>
    concurrencyLimit: 2            # optional, default 2
```

Validation rules:

- `owner` and `namespace` are unique across the registry.
- `namespace` is a valid DNS-1123 label. When the owner is not (for example an uppercase login), the registry uses an explicit lowercase alias. The namespace is never derived from the owner at runtime.
- `repos[].name` is a valid DNS subdomain and unique within the tenant namespace.
- `repos[].url` is unique within the tenant namespace.
- `tier` is `build` or `deploy`.
- `developers` may be empty. Email addresses are personal data; do not commit them to a public repository if that is a concern.
- The registry is applied with GitOps or `helmfile apply`; there is no other list of namespaces or repositories.

## GitHub authentication

- One **public** GitHub App, because a private App can only be installed on its owner account.
- One PaC controller and one App private key, stored only in the `pipelines-as-code` namespace.
- Each installation is one tenant's authorization grant. PaC derives an installation token from the `installation.id` in the webhook; installation tokens are scoped per owner and rate-limited per installation.
- The App's webhook endpoint is the gosmee relay until a domain and Cloudflare Tunnel exist; the switch is an App setting, not a design change.
- The App subscribes to `check_run`, `check_suite`, `commit_comment`, `issue_comment`, `pull_request` and `push`. The `repository` event is optional because auto-configure is off.
- No personal access token. A fine-grained PAT cannot create check runs, and PAT mode requires per-repository webhooks; it is not an alternative.
- Unknown installations are inert: no Repository CR is created, so no run starts.
- Per-owner Apps and controller shards are a future option only for key isolation. They do not replace the network sandbox.

## Per-tenant objects

For every registry entry the platform must produce:

| Object | Contract |
|---|---|
| `Namespace` | Name = `namespace`; labels `home.cluster/tenant=<owner>`, `home.cluster/tier=<tier>` and `home.cluster/managed=true` (the pruner's marker); Pod Security enforce/audit/warn = `restricted`; enforce-version `latest`. |
| `ResourceQuota` | Per-tier CPU, memory, ephemeral storage, pod count, PVC count and storage. Tenant-level overrides are allowed. |
| `LimitRange` | Per-tier default requests and limits for containers. |
| `CiliumNetworkPolicy` | Default-deny ingress and egress with the allows and denies in *Network policy*. |
| `ServiceAccount` `tekton-bot` | Exists in every tenant namespace; no RoleBinding and no automounted token. |
| `ConfigMap` `npm-npmrc` | Points at the Verdaccio cache; the `npm-install-v1` task reads it as a workspace. |
| `Repository` CR per repo | Namespace = tenant namespace; name = `repos[].name`; `spec.url`; `spec.concurrency_limit`; `spec.settings.pipelinerun_provenance: source`. |
| Developer RBAC | One `RoleBinding` per developer per tenant namespace to the read-only `tekton-developer` ClusterRole, plus one `ClusterRoleBinding` per unique developer to `tekton-namespace-lister`. |

Cluster-scoped objects:

- `tekton-developer` ClusterRole: `get/list/watch` on `pipelineruns`, `taskruns`, `pipelines`, `tasks`, `customruns` (`tekton.dev`), `repositories` (`pipelinesascode.tekton.dev`), `pods`, `events` (`""`), and `get` on `pods/log` (the log subresource has no list/watch). No secrets, no configmaps, no create/update/delete/patch, no exec/attach/port-forward.
- `tekton-namespace-lister` ClusterRole: `get/list` on `namespaces` (metadata for the dashboard dropdown).
- `ValidatingAdmissionPolicy` and binding: see *Admission control*.

`tekton-pipelines` remains the control-plane namespace: shared tasks, Tekton configuration and shared objects only. It holds no tenant Repository CR.

The deploy tenant namespace additionally holds the operator-owned
`home-terraform-plans` PVC and its cleanup CronJob: the terragrunt plan/apply
handoff must sit next to the deploy runs, and a CronJob cannot mount a PVC from
another namespace. These are platform maintenance objects, not tenant-supplied
ones; build tenants never get them.

## Namespace contract

- The Repository CR namespace is the run namespace. There is no separate PaC setting for this.
- A tenant namespace must exist before any Repository CR in it; the platform creates both in the same apply.
- A tenant namespace must contain only:
  - the `tekton-bot` service account,
  - the `npm-npmrc` ConfigMap,
  - PaC-managed per-run git auth secrets,
  - tenant-supplied secrets (registry credentials, if any),
  - tenant PipelineRuns, TaskRuns, pods and PVCs.
- A tenant namespace must never contain the operator's Docker Hub, GHCR, AWS or GitOps credentials. Secret mounts bypass RBAC, so any secret in the namespace is readable by that tenant's pipelines.
- `build` tenants receive no deploy credentials. `deploy` tenants may receive the credentials their GitOps path needs.

## Network policy

Cilium is required. A `NetworkPolicy` on a Flannel cluster is accepted and ignored, so the policy is only effective once Cilium is the CNI.

For each tenant namespace, the `CiliumNetworkPolicy` must:

**Ingress:** deny everything except `fromEntities: host` (kubelet probes and host access).

**Egress allow:**

- cluster DNS: `kube-system`, `k8s-app=kube-dns`, UDP/TCP 53;
- Verdaccio: namespace `verdaccio`, TCP 4873;
- `world` (public internet);
- for `deploy` only: `kube-apiserver`.

**Egress deny (takes precedence over `world`):**

- `192.168.0.0/16` (home LAN and node IPs);
- `172.16.0.0/12`;
- `169.254.0.0/16` (link-local and metadata).

The pod and service CIDRs are intentionally not denied: that would also deny DNS and Verdaccio. Other namespaces and the API server are denied by default because they are cluster IPs and are not in the allow list.

Residual behavior: a `build` tenant can reach the public internet (needed for git, package downloads and registry pushes). Quotas bound resources, not outbound traffic.

## RBAC

**Viewer identity** is the Kubernetes user a dashboard request is evaluated as: the Cognito email injected by oauth2-proxy as `Impersonate-User`. **Workload identity** is the `tekton-bot` service account the PipelineRun pods run as. They are independent:

- A viewer without `secrets` in their Role cannot see secrets in the dashboard.
- A pipeline can still mount any secret in its namespace, regardless of the service account's RBAC.
- A developer who can create a PipelineRun in a namespace can use that namespace's workload identity. Developers are therefore read-only.

Rules:

- `tekton-developer` is read-only as specified above.
- Developer bindings are per tenant namespace and bind `User: <email>` (the proxy does not inject groups).
- Admin read/write stays cluster-wide and is reserved for cluster managers; it is never granted to tenant users.
- The dashboard runs with `--namespaces` empty so impersonation plus RBAC decides which namespaces and resources are visible.
- A tenant developer may see namespace names (metadata) but not resources in namespaces where they have no RoleBinding.

## Admission control

A `ValidatingAdmissionPolicy` applies to `pipelinesascode.tekton.dev/v1alpha1` `Repository` CREATE and UPDATE:

- Derive the owner from `spec.url` (the fourth path segment, e.g. `https://github.com/acme/widgets` → `acme`).
- Require `namespaceObject.metadata.labels["home.cluster/tenant"]` to exist.
- Require the lowercased URL owner to equal the lowercased label value.
- Exempt the `pipelines-as-code` namespace, which is reserved for PaC's global Repository CR (no global CR exists in this cluster today).
- `validationActions: Deny`; `failurePolicy: Fail`.

This is defense-in-depth. The primary control is that only the platform creates Repository CRs; tenants cannot write them.

## PR provenance and trust

- `settings.pipelinerun_provenance: source` for every tenant. A pull request runs the `.tekton/` definition from its own branch, so tenants can iterate on their pipeline in a feature branch.
- `/ok-to-test` and `require-ok-to-test-sha` remain enabled for non-members.
- The safety boundary is the namespace sandbox, not the provenance setting. A PR author can change the pipeline; the pipeline can only use its tenant namespace, its quota, its network policy and its secrets.
- A tenant is trusted to run arbitrary code in its own namespace. It is not trusted with other tenants, the home LAN, the Talos API or (for `build`) the Kubernetes API.

## Secrets

- PaC creates a per-run git auth secret in the run namespace, scoped to the triggering repository.
- A tenant that pushes container images supplies its own registry secret. The operator creates it out-of-band; it is never committed and never reused from the operator's credentials.
- `npm-npmrc` is non-secret configuration.
- `deploy` credentials exist only in `deploy` tenant namespaces. `build` namespaces contain no deploy credentials.
- The namespace is the secret boundary. RBAC on secrets protects the dashboard view, not the workload.

## Tiers

| Property | `build` | `deploy` |
|---|---|---|
| Registry credentials | tenant-supplied | operator-supplied |
| Deploy credentials | none | as required by the GitOps path |
| Kubernetes API from pods | denied | allowed |
| Pod Security | `restricted` | `restricted` |
| Egress | DNS, Verdaccio, internet | DNS, Verdaccio, internet, API server |
| Developer role | read-only | read-only |

## Migration

Target state: no tenant Repository CR in `tekton-pipelines`; every tenant has a namespace; auto-configure is off; Cilium enforces the network policy.

Phases:

1. **Cilium** (base layer): replace Flannel with Cilium, keep kube-proxy, verify `CiliumNetworkPolicy` enforcement. Existing pods restart.
2. **Platform objects**: apply the tenant namespaces, quotas, PSA labels, network policies, service accounts, `npm-npmrc`, Repository CRs and developer bindings for the operator's two owners and their repositories.
3. **PaC settings**: set `auto-configure-new-github-repo: "false"`; retire the PaC repositories chart and its release.
4. **Cut over**: remove the old Repository CRs from `tekton-pipelines`. Runs now start in tenant namespaces.
5. **Task compliance**: replace kaniko with a rootless builder and audit every task image for `restricted` PSA (non-root, drop ALL capabilities, `RuntimeDefault` seccomp — the rootless build step may use a `Localhost` profile because `RuntimeDefault` blocks the user-namespace syscalls it needs).
6. **Docs and sync**: replace the three-way namespace sync with the registry; update the PaC, shared-pipelines and RBAC docs.

Rollback: each phase is reversible; the Cilium rollback is the only cluster-wide step.

Acceptance criteria:

- Adding a registry entry and applying produces exactly one namespace with the expected labels, quota, LimitRange, network policy, service account, `npm-npmrc`, Repository CRs and developer bindings.
- Removing the entry prunes the same objects (Longhorn volumes may require explicit deletion).
- An unregistered owner that installs the App produces no namespace and no Repository CR.
- A `build` tenant cannot reach the API server, the home LAN or another tenant namespace; it can reach DNS, Verdaccio and the internet.
- A `deploy` tenant can reach the API server.
- A developer can see their own PipelineRuns, TaskRuns, pods and logs; cannot see secrets or configmaps; cannot create, update, delete or exec; cannot see resources in another tenant namespace.
- An admin sees every tenant.
- A PR can change `.tekton/` and run; `restricted` PSA is enforced.
- No operator credentials exist in a `build` namespace.
- No real secrets or personal data are committed to Git.

## Risks

- **Single App key.** One private key can mint tokens for every installation. A leak is contained by uninstalling the App and rotating the key, not by tenant separation. The App is owned by a personal account; transferring it to the operator organization reduces account-lifecycle risk.
- **Shared CI node.** Quotas and PSA bound containers, but a kernel or runtime escape is cross-tenant. Only a separate node, VM or cluster removes this.
- **Internet egress.** `build` tenants can reach the public internet; quotas do not bound network traffic.
- **Cilium rollout.** The CNI change is cluster-wide and restarts pods; it must be a separate change with its own rollback.
- **`restricted` PSA.** It will break root-running task images, including kaniko; the rootless builder and task audit are part of the migration, not optional.
- **Manual registry.** New repositories require a registry entry; there is no auto-discovery. This is the price of Git being the source of truth.
- **Namespace deletion.** Removing a tenant deletes the namespace and its objects; Longhorn volumes may survive and need explicit cleanup.
- **Public App.** Anyone can install the App; the registry gate makes unknown installations inert, but the App still has access to repos where it is installed until it is uninstalled.
- **Personal-account App ownership.** If the owning account is lost, every installation and the App credentials are lost. Transfer to the organization is recommended.

## References

- [pac-tenancy-adr.md](../../../architecture-decision-log/home/pac-tenancy-adr.md)
- [cilium-networkpolicy-adr.md](../../../architecture-decision-log/home/cilium-networkpolicy-adr.md)
- [GLOSSARY.md](../../../GLOSSARY.md)
- [pac-run-namespace-adr.md](../../../architecture-decision-log/home/pac-run-namespace-adr.md) — the superseded single-namespace decision
