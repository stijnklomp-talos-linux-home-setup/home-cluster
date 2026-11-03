<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-01 | Last verified: 2026-11-01 -->

# AGENTS.md (home-istio)

Day-0 base component: Istio 1.31.3 control plane + ambient data plane (no sidecars) and the Gateway API CRDs. Installed after Cilium; before any ambient namespace label or edge Gateway.

## Overview
Overview of this component — see `README.md` for prose.

## Setup (deploy & teardown)

| Task | Command |
|------|---------|
| Prerequisite | Cilium with `cni.exclusive: false` applied and `ds/cilium` rolled out (`../home-cilium`) |
| Apply | `helmfile apply` (first install on a fresh cluster: `helmfile apply --skip-diff-on-install`, see README) |
| Verify | `./verify-ambient.sh` (asserts mTLS success + plaintext denial) |
| Edge Gateway | `EDGE_LB_IP=<lb-ip> EDGE_HOST_SUFFIX=<lb-ip>.sslip.io helmfile -f helmfile-edge.yaml.gotmpl apply` (production edge; separate from `helmfile apply`) |
| Edge cleanup | `helmfile -f helmfile-edge.yaml.gotmpl destroy` (releases the MetalLB address); do this **before** `home-istio` itself |
| Analyze | `docker run --rm -v "<workspace>/home-cluster-1-config:/cfg:ro" docker.io/istio/istioctl:1.31.3 --kubeconfig /cfg/kubeconfig analyze -A` |
| Sandbox cleanup | `helmfile -l stage=sandbox destroy` — not permanent: the next `helmfile apply` recreates both sandbox releases |
| Cleanup | `helmfile -l 'stage!=crds' destroy` (remove ambient namespace labels first). Plain `helmfile destroy` also uninstalls `gateway-api-crds` and deletes every Gateway/HTTPRoute/TLSRoute cluster-wide — only when abandoning Istio |

## Commands
Full command table lives in the Setup section; offline validation: `helm lint ./charts/upstream/istiod-1.31.3.tgz -f values/istiod-values.yaml` and `helm template`, via the alpine/helm container (see workspace root AGENTS.md).

## Key files

| File | Notes |
|------|-------|
| `charts/upstream/{base,istiod,cni,ztunnel}-1.31.3.tgz` | vendored from the 1.31.3 release tarball (the Helm repo lags); never edit in place |
| `values/istiod-values.yaml` | `PILOT_ENABLE_AMBIENT`, HBONE metadata, one replica, reduced requests |
| `values/cni-values.yaml` | `ambient.enabled: true`; the chart defaults leave ambient off |
| `values/ztunnel-values.yaml` | per-node L4 proxy, reduced requests |
| `kustomizations/gateway-api/standard-install.yaml` | Gateway API v1.5.1 standard channel (vendored) |
| `manifests/istio-system/namespace.yaml` | raw manifest (not a kustomization) applied by the `istio-base` presync hook: privileged PodSecurity label for `istio-system` |
| `kustomizations/ambient-probe-policy/probe-policy.yaml` | CCNP for SNAT-ed kubelet probes from `169.254.7.127/32` |
| `kustomizations/ambient-sandbox/` | ticket #10 proof: ambient namespace with `PeerAuthentication` STRICT (PodSecurity baseline), nginx + curl |
| `kustomizations/edge-route-namespaces/` | `istio.io/edge-route: enabled` label on the five route namespaces; applied by the edge helmfile presync hook (Helm cannot adopt namespaces owned by other components) |
| `charts/edge-gateway/` | production edge: `Gateway` `istio-internal` (HTTPS terminate ×3 + TLS passthrough ×2 + HTTP redirect), multi-SAN `Certificate` `internal-tls`, redirect HTTPRoute |
| `helmfile-edge.yaml.gotmpl` | separate helmfile for the edge; real address/host suffix come from `EDGE_LB_IP` / `EDGE_HOST_SUFFIX`, committed defaults are TEST-NET-1 |
| `verify-ambient.sh` | acceptance test; needs docker + the workspace kubeconfig |

## Conventions & rules

- Deploy after Cilium. The Istio CNI plugin is chained into the Cilium config; `cni.exclusive: true` on Cilium silently removes it and breaks ambient redirection.
- Keep `kubeProxyReplacement: false` and `bpf.masquerade: false` in Cilium. Istio's ambient docs explicitly do not support Cilium BPF masquerading (kubelet health checks break).
- The CCNP must keep `enableDefaultDeny: {ingress: false, egress: false}`; without it the empty `endpointSelector` turns the policy into a cluster-wide default deny.
- Ambient enrollment is a **namespace** label (`istio.io/dataplane-mode: ambient`). Do not put Istio sidecars in ambient namespaces; do not label `kube-system`, `argo`, `tekton-pipelines` or `pinniped-system` in this ticket.
- Do not enable Cilium L7 policy alongside Istio L7 policy (ADR); Cilium remains L3/L4.
- `istio-system` is privileged only because ztunnel/CNI need `NET_ADMIN`/`SYS_ADMIN`; do not copy that label to application namespaces.
- The production edge Gateway lives in its own helmfile because the address and host suffix are environment values. Never add it to `helmfile.yaml`, never commit real LAN addresses/hostnames, and never point it at an address another Service owns.
- Teardown is CRD-safe by default: `helmfile -l 'stage!=crds' destroy`. Plain `helmfile destroy` uninstalls `gateway-api-crds`, which deletes all Gateway API CRDs cluster-wide and cascades every Gateway/HTTPRoute/TLSRoute; use it only when abandoning Istio. Destroy the edge release first so it releases the address while the CRDs still exist.
- The `.gotmpl` suffix matters: every command is `helmfile -f helmfile-edge.yaml.gotmpl ...`; a file named `helmfile-edge.yaml` does not exist.
- The gateway pod has **no node affinity** (ingress-nginx used to be pinned to the workers). It can schedule on the CI node; accepted risk, documented in README. Pinning would need a `GatewayClass.spec.parametersRef` ConfigMap, which this repo does not own.
- The sandbox releases are part of `helmfile.yaml`: `-l stage=sandbox destroy` removes them only until the next `helmfile apply`. Retire them by deleting the releases from the helmfile.
- Routes are owned by the component that owns the namespace: `cicd-argocd` (`argo`), `cicd-tekton-pipelines-as-code` (`pipelines-as-code`), `home-sso-auth-rbac` (`home-sso-auth`, `pinniped-system`). This component owns only the Gateway, the Certificate, the redirect route and the namespace labels.
- `allowedRoutes` is a namespace label selector (`istio.io/edge-route: enabled`). Never label tenant namespaces; tenants have no edge routes by design.
- Pinniped is TLS **passthrough** (`tls-login`, `tls-kubectl`): the Gateway must never reference `supervisor-public-tls` or the impersonation proxy secret; those services terminate TLS themselves.
- The ticket #11 temporary gateway was retired after the #12 cutover. Do not reintroduce `istio-temp` or a second Gateway on the production address.
- Never edit the vendored `.tgz` or `standard-install.yaml`; refresh from the documented release URLs.
- Cross-repo rules: workspace root `AGENTS.md`.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- The Gateway API CRDs and Istio charts are upstream artifacts; regenerate, never hand-edit.
- `<lb-ip>`, `<your-github-org>`, `<region>`, `<user-pool-id>` are TODOs, not values to invent.

## Checklist
- [ ] helmfile chart paths resolve (no dangling `./charts/...`)
- [ ] `helm lint` and `helm template` pass for every vendored chart + values file
- [ ] `helmfile -f helmfile-edge.yaml.gotmpl template` renders with defaults and with `EDGE_*` overrides
- [ ] `kubectl kustomize kustomizations/<name>` parses for every kustomization (raw manifests live in `manifests/`)
- [ ] Gateway listeners are still six, with five distinct hostnames; every 443 route binds by `sectionName`
- [ ] No tenant namespace carries `istio.io/edge-route`
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value

## Examples
`home-metallb` is the reference helmfile pattern and shows the presync namespace hook used here.

## When stuck
- `kubectl -n istio-system logs ds/ztunnel`, `... ds/istio-cni-node`, `... deploy/istiod`.
- Ambient troubleshooting: `istioctl ztunnel-config all`, `istioctl analyze -A`.
- Cutover and rollback: `developer/docs/operations/istio-ambient-cutover.md`.
