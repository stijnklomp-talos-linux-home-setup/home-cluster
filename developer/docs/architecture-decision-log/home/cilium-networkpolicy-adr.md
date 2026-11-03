# CNI: Cilium to enforce tenant network isolation

## Status

Accepted (2026-10)

## Context

The cluster ran Flannel. Flannel provides pod networking but does not implement the Kubernetes `NetworkPolicy` API, so any `NetworkPolicy` in the cluster is accepted and ignored. With one trusted user that was irrelevant. With invited external GitHub owners running arbitrary `.tekton/` code it is the difference between a namespace boundary and a suggestion: a tenant pod can reach every other pod and service, the home LAN, the Talos API and the internet.

Pod Security `restricted`, ResourceQuotas and RBAC bound what a pipeline can do through the Kubernetes API; they do not bound what its network namespace can reach. A separate GitHub App or controller does not change the network either.

## Decision

- Replace Flannel with **Cilium** as the CNI. Keep `kube-proxy` (no kube-proxy replacement) to limit the change.
- Every tenant namespace gets a default-deny `CiliumNetworkPolicy` that allows ingress only from the host (kubelet probes) and egress only to:
  - cluster DNS,
  - the Verdaccio npm cache,
  - the public internet.
- The policy denies egress to RFC1918 ranges (including the home LAN and node IPs) and link-local/metadata addresses. Cilium deny rules take precedence over the `world` allow.
- `build` tenants may not reach the Kubernetes API server. `deploy` tenants may (`toEntities: kube-apiserver`) because their GitOps tasks need it.
- Tenants remain on the shared CI node with per-namespace quotas and LimitRanges; node-level isolation is explicitly out of scope for this decision.

## Rationale

- Cilium is the supported policy-capable CNI path on Talos, enforces both `NetworkPolicy` and `CiliumNetworkPolicy`, and supports the private-range deny plus `world` allow combination the tenant policy needs.
- Calico in policy-only mode alongside Flannel (Canal) was the alternative; it preserves Flannel but adds a second networking stack and a less common Talos configuration.
- Separate clusters or VMs per tenant would also isolate the kernel, but they multiply control planes and were rejected for this iteration.
- Accepting the flat network was rejected: it makes "a compromised `.tekton/` in one organization must not affect the others" false.

## Consequences

- **Easier:** tenant egress policy is declarative; build tenants cannot reach the LAN, other tenants or the API server; the policy is per namespace, so adding a tenant needs no CNI change.
- **Harder:** Cilium is a base-layer rollout (Talos `cni: none`, Cilium install, pod restarts); Cilium CRDs become a dependency of the tenant chart; Hubble is optional but useful for debugging.
- **Residual risk:** a `build` tenant may still reach the public internet (needed for package and image access); quotas bound resources, not outbound traffic. A kernel or runtime escape on the shared CI node is not contained.
