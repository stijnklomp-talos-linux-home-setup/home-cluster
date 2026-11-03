# Service networking: Istio edge, ambient mTLS, optional GraphQL federation

## Status

Accepted (2026-11-01). Implemented the same day (tickets #10–#12): Istio
ambient and the internal Gateway `istio-internal` serve ArgoCD, Tekton
Dashboard, PaC and Pinniped on the former ingress-nginx address, and
`ingress-nginx` has been removed. The only refinement over this decision:
`allowedRoutes` is a namespace label selector (`istio.io/edge-route: enabled`)
rather than the default `All`, so no tenant namespace can attach a route.

## Context

- `home-cluster-1` exposes its UIs through `ingress-nginx` on a MetalLB L2 address. The project retired in March 2026: chart 4.15.1 is the final release, with no bugfixes or CVE fixes. The L7 edge must be replaced.
- The CNI is Cilium 1.20.4 with kube-proxy retained (`kubeProxyReplacement: false`), VXLAN, and `bpf.masquerade: false`. MetalLB owns LoadBalancer addresses; cert-manager issues LAN certificates from `home-issuer`; Cognito + Pinniped + oauth2-proxy provide authentication.
- New workloads are a Next.js UI and Rust services. REST/OpenAPI is the direct service contract; GraphQL is wanted as an aggregation path for the UI and for services that need data from several services in one call.
- Requirements: service-to-service mTLS; default-deny exposure; public access only for explicitly routed UIs later; no L7 traffic-splitting requirement; one cluster; small scale; no production users during the cutover.
- The cluster has no Prometheus/Grafana/OTel stack, and Hubble is off.

## Decision

1. **Edge.** Replace `ingress-nginx` with an Istio 1.31.x ingress gateway using Gateway API: `HTTPRoute` for HTTP(S) routes and `TLSRoute` for Pinniped's TLS passthrough. One internal gateway on the existing MetalLB address; public exposure is deferred. ArgoCD, Tekton Dashboard, PaC and Pinniped move to it.
2. **Mesh.** Enable Istio ambient mode for application namespaces and the gateway namespace. ztunnel provides mTLS and SPIFFE identity for direct service-to-service calls; no waypoint in iteration 1. Cilium remains the L3/L4 policy and tenant-isolation layer. Do not use Cilium L7 policy alongside Istio L7 policy.
3. **Service contract.** Every service publishes an OpenAPI 3.1 contract and Pact tests. GraphQL is opt-in through a `graphql` feature in the Rust template; when enabled, the service is an Apollo Federation v2 subgraph. REST remains the direct path; the GraphQL gateway is the aggregation path. One template, two optional API shapes.
4. **GraphQL gateway.** Hive Gateway 2.15.x serves a supergraph composed by `rover supergraph compose`. The supergraph is a GitOps-managed ConfigMap mounted as a directory and polled by Hive; schema changes do not need a gateway image rebuild. The gateway is ClusterIP-only. One replica, no PDB, readiness/liveness probes. The gateway authenticates JWTs; every service validates the JWT again; field-level authorization lives in the Rust services, with `@requiresScopes` for coarse scopes. No trusted identity headers across service boundaries.
5. **Repositories and deployment.** `home-graphql` owns the gateway, `supergraph.yaml`, and the supergraph artifact; `home-cluster` holds the service declaration. Istio lives in a new `home-istio` base component managed by helmfile (base, istiod, cni, ztunnel; Gateway API CRDs). The Gateway API `Gateway` provisions its own Deployment/Service.
6. **Cutover.** Big-bang replacement, reusing the current LoadBalancer address, with a documented rollback. The runbook is [operations/istio-ambient-cutover.md](../../operations/istio-ambient-cutover.md).
7. **Cilium changes.** Set `cni.exclusive: false` for Istio CNI chaining; add a cluster-wide policy allowing ambient host probes from `169.254.7.127/32`; keep `kubeProxyReplacement: false` and `bpf.masquerade: false`.

## Rationale

- **Why Istio at the edge.** `ingress-nginx` is EOL. Cilium Gateway API would require enabling kube-proxy replacement, reversing the Cilium ADR, and still lacks native JWT validation. Istio Gateway gives Gateway API routing, JWT via `RequestAuthentication`, and a path to ambient mTLS with one control plane. The learning goal is a deliberate secondary benefit.
- **Why ambient rather than sidecars.** mTLS is a hard requirement. Ambient avoids per-pod sidecars and fits the existing Cilium configuration: kube-proxy retained, BPF masquerade off, VXLAN. Talos has no official Istio support statement, so the cutover is staged and reversible.
- **Why not GraphQL Mesh.** Services expose GraphQL natively; Mesh's per-service REST/gRPC adapters add a layer without value. Federation is the standard composition model, and `async-graphql` covers Federation v2. Hive Gateway is the runtime; Mesh is not needed.
- **Why REST stays.** OpenAPI is the contract direct callers and external tooling can inspect; GraphQL is an optional aggregation facade. One Rust template with an optional `graphql` feature avoids a second template and keeps the existing OpenAPI/Pact investment.
- **Why one replica and no PDB.** At this scale, a PDB cannot provide availability with one replica; it would only block node drains. Brief downtime during image rollouts is accepted.
- **Why a ConfigMap supergraph.** The gateway image depends on every subgraph schema; baking it would force a gateway rebuild and a one-replica rollout for every schema change. ConfigMap polling decouples schema changes from gateway releases and keeps the supergraph reviewable as a Git diff.

## Consequences

- **Easier:** one L7 edge; automatic mTLS for direct calls; one aggregation endpoint; OpenAPI remains available; public exposure becomes an explicit route decision; schema changes do not require a gateway image build.
- **Harder:** Istio upgrade cadence (roughly every three months); Cilium CNI chaining and a cluster-wide probe policy; one-replica gateway downtime; the supergraph is a mutable ConfigMap; no breaking-change gate; Talos + Istio is not an officially supported combination; Pinniped's TLS passthrough on a shared 443 listener needs verification; the GraphQL gateway cannot identify the calling service without a waypoint.
- **Scope:** only GraphQL-enabled services join the supergraph. REST-only services remain direct-only. Public access, metrics/traces, waypoint authorization, schema automation and EKS remain out of scope for this iteration.

## Deferred

Tracked as checkboxes in the cutover runbook:

- Public access: domain, Cloudflare Tunnel, DNS-01 issuer, public Gateway, route allowlist, public TLS.
- Observability: Istio metrics, Hive OpenTelemetry, Prometheus/Grafana/OTel stack.
- PDB / second gateway replica.
- Schema automation from service CI and a breaking-change gate.
- Waypoint and per-caller service authorization.
- EKS portability (AWS-specific edge: NLB/ALB, VPC CNI, EBS CSI).
