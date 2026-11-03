# PaC tenancy: one run namespace per GitHub owner

## Status

Accepted (2026-10). Supersedes [pac-run-namespace-adr.md](./pac-run-namespace-adr.md). Spec: [tenancy spec](../../cicd/pipelines/tenancy/spec.md).

## Context

Pipelines-as-Code (PaC) runs a PipelineRun in the namespace of the Repository CR that matched the webhook. The 2026-09 decision collapsed every repository into the single `tekton-pipelines` namespace because the cluster had one user and team isolation was pure overhead.

The cluster now serves multiple GitHub owners (personal accounts and organizations) from one public GitHub App, with invited onboarding rather than open registration. Three properties of the old model break at that point:

- A namespace is the only Kubernetes-native authorization boundary, and RBAC cannot filter by label. Everything in one namespace is visible to anyone with access to that namespace.
- A PipelineRun runs with the namespace's service account and can mount any secret in that namespace; secret mounts are authorized by the kubelet, not by RBAC. A shared `tekton-pipelines` therefore lets every tenant's `.tekton/` read every pipeline secret.
- The cluster CNI was Flannel, which does not enforce `NetworkPolicy`, so a pod could reach other namespaces, the home LAN and the internet regardless of namespace. A namespace is an API boundary until a policy-capable CNI is installed (see [cilium-networkpolicy-adr.md](./cilium-networkpolicy-adr.md)).

The three duplicated namespace lists (`developer/pipelines/values/teams.yaml`, the PaC repositories chart and the cluster RBAC chart) also made adding an owner a multi-file edit.

## Decision

- **Tenant = GitHub owner.** One tenant namespace per owner. A registry (schema in the [tenancy spec](../../cicd/pipelines/tenancy/spec.md)) is the single source of truth for `owner → namespace`, tier, repositories and developer emails. Namespaces use an explicit mapping: `stijnklomp-LAVE → stijnklomp-lave`.
- **The Repository CR namespace is the run namespace.** `tekton-pipelines` keeps shared tasks, Tekton configuration and control-plane objects only; it holds no tenant Repository CRs.
- **One public GitHub App, one controller.** The App key lives only in `pipelines-as-code`; installations are the unit of GitHub authentication and rate limiting. Per-owner Apps and controller shards are a future option only if a tenant requires its own key.
- **PaC auto-configure is disabled.** PaC does not create namespaces or Repository CRs; the registry is the complete list. A `ValidatingAdmissionPolicy` additionally requires a Repository CR's URL owner to match its namespace's `home.cluster/tenant` label.
- **Two capability tiers.** `build` (external tenants: tenant-supplied registry credentials, no deploy credentials, no Kubernetes API access) and `deploy` (owners the cluster operator controls: deploy credentials and API access for the GitOps path).
- **`source` provenance for every tenant.** PaC runs the `.tekton/` definition from the event branch, so tenants can change their pipeline in a feature branch. The safety boundary is the sandbox, not the provenance setting.
- **`restricted` Pod Security everywhere** in tenant namespaces. The rootless builder replaces kaniko.
- **Network isolation** is provided by Cilium and a per-tenant default-deny `CiliumNetworkPolicy` (see the Cilium ADR).
- **Developers are read-only.** A `tekton-developer` ClusterRole grants get/list/watch on Tekton resources, pods, logs, events, Repository CRs and namespace metadata; no secrets, no configmaps, no create/update/delete, no exec. Admin read/write stays cluster-wide and is reserved for cluster managers. The dashboard's `--namespaces` pin is removed so impersonation plus RBAC decides visibility.
- **Shared CI node accepted.** Quotas and LimitRanges bound each tenant, but a kernel or runtime escape remains a cross-tenant risk.

## Rationale

- One namespace per owner is the smallest unit that maps to GitHub's ownership model and bounds the namespace count. Per-repo namespaces multiply the plumbing (service account, ConfigMap, quota, policy, RBAC) without adding a boundary that GitHub does not already provide.
- A complete registry keeps Git the source of truth; auto-configure would create cluster-only objects that a rebuild could not restore. The cost is one registry line per repository.
- One App avoids N controller shards (tech preview) and the per-owner secrets the original design tried to avoid. The blast radius of the single key is recorded as an accepted risk.
- `source` provenance is compatible with feature-branch `.tekton/` changes only if the workload cannot reach other tenants. Cilium, `restricted` PSA, quotas and per-tenant secrets provide that boundary; `default_branch` would be a substitute for a missing sandbox, at the cost of the developer workflow the operator wants.
- The old single-namespace decision was correct for a single trusted user; it is superseded because the trust model changed, not because the reasoning was wrong.

## Consequences

- **Easier:** one registry to add or remove a tenant; per-tenant quotas, secrets and visibility; no shared pipeline credentials in tenant namespaces; GitHub checks remain the developer-facing feedback.
- **Harder:** per-tenant plumbing must be generated; Cilium is a cluster-wide prerequisite; existing tasks must run as non-root; `build` tenants cannot use `kubectl` and cannot read other namespaces; the operator manages the registry; the App key covers every installation; the shared CI node remains a shared kernel.
- **Migration:** the operator's two owners move out of `tekton-pipelines` into tenant namespaces, the PaC repositories chart is retired, and the three-way namespace sync is replaced by the registry.
- **App ownership:** the App is owned by a personal account, so every installation depends on that account surviving. Transferring the App to the operator organization is recommended.
