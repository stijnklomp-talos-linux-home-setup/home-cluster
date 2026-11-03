# home-istio

Istio 1.31.3 for `home-cluster-1`: the internal edge and ambient mTLS. This
component installs the control plane, the ambient data plane and the production
edge `Gateway`. The per-service `HTTPRoute`/`TLSRoute` objects live in the
repos that own their namespaces (`cicd-argocd`, `cicd-tekton-pipelines-as-code`,
`home-sso-auth-rbac`).

Ambient mode has no sidecars: a namespace opts in with
`istio.io/dataplane-mode: ambient`, the Istio CNI node agent redirects its pod
traffic to the per-node `ztunnel`, and ztunnel provides mTLS and SPIFFE
identity. Cilium stays the L3/L4 policy layer. See the service-networking ADR
and `developer/docs/operations/istio-ambient-cutover.md`.

## What gets installed

| Release | Chart | Purpose |
|---------|-------|---------|
| `gateway-api-crds` | vendored upstream YAML | Gateway API v1.5.1 standard channel; the edge Gateway needs it |
| `istio-base` | `base-1.31.3.tgz` | Istio CRDs, cluster roles, `istio-system` setup |
| `istiod` | `istiod-1.31.3.tgz` | control plane; `PILOT_ENABLE_AMBIENT` on |
| `istio-cni` | `cni-1.31.3.tgz` | chained CNI plugin; installs ambient redirection |
| `ztunnel` | `ztunnel-1.31.3.tgz` | per-node L4 mTLS proxy |
| `ambient-probe-policy` | kustomization | Cilium CCNP allowing SNAT-ed kubelet probes |
| `ambient-sandbox` | kustomization | ticket #10 proof: nginx server + curl client inside the mesh (`PeerAuthentication` STRICT) |
| `ambient-sandbox-nomesh` | kustomization | ticket #10 proof: identical client outside the mesh |
| `edge-gateway` | `charts/edge-gateway` | production Gateway `istio-internal`, multi-SAN Certificate, HTTP->HTTPS redirect (separate helmfile) |

`helmfile apply` does **not** install the edge Gateway: it needs the real
MetalLB address and host suffix, which never enter the public repo. That
release lives in `helmfile-edge.yaml.gotmpl` and is described below.

## Prerequisites

Cilium must run with `cni.exclusive: false` first, otherwise it rewrites
`/etc/cni/net.d` and removes the Istio CNI plugin. In `home-cilium`:

```sh
helmfile apply
kubectl -n kube-system rollout status ds/cilium
```

`istio-system` is labeled `pod-security.kubernetes.io/enforce: privileged` by
a presync hook. ztunnel and the CNI agent need `NET_ADMIN`/`SYS_ADMIN`, which
the cluster-wide `baseline` default rejects (Talos exempts only `kube-system`).

## Deploy

```sh
helmfile apply
```

On a **fresh cluster** the first run needs `helmfile apply --skip-diff-on-install`:
the sandbox release contains a `PeerAuthentication`, and helmfile's diff phase
runs before the CRDs are installed, so plain `apply` would fail with
`no matches for kind "PeerAuthentication"`. Later runs diff normally.

Charts are vendored because the upstream Helm repository stops at
`1.31.0-rc.0` for this line. Refresh from a release tarball:

```sh
tar xzf istio-<version>-linux-amd64.tar.gz
helm package istio-<version>/manifests/charts/base \
             istio-<version>/manifests/charts/istio-control/istio-discovery \
             istio-<version>/manifests/charts/istio-cni \
             istio-<version>/manifests/charts/ztunnel -d charts/upstream
```

`istiod` maps to `istiod-<version>.tgz` (the chart name, not the directory).
Refresh the Gateway API CRDs with the `curl` line in
`kustomizations/gateway-api/kustomization.yaml`.

## Verify

```sh
./verify-ambient.sh
```

The script asserts that `istiod`/`ztunnel`/`istio-cni-node` are ready, that
`istioctl analyze -A` is clean, that the in-mesh client gets HTTP 200, that
the same request from the unlabeled `ambient-sandbox-nomesh` namespace is
denied, and that ztunnel lists the server as an `HBONE` workload.

Manual checks:

```sh
docker run --rm -v "<workspace>/home-cluster-1-config:/cfg:ro" \
  docker.io/istio/istioctl:1.31.3 --kubeconfig /cfg/kubeconfig analyze -A
kubectl -n istio-system get pods
kubectl get namespace ambient-sandbox --show-labels
```

## Production edge (ticket #12)

`helmfile-edge.yaml.gotmpl` plus `charts/edge-gateway/` deploy the permanent
`Gateway` `istio-internal` in `istio-ingress`. It replaced ingress-nginx in
the big-bang cutover and reuses the address ingress-nginx owned. Gateway API
objects in the owning namespaces replaced every Ingress:

| Host | Listener | Route | Backend |
|------|----------|-------|---------|
| `argocd.<suffix>` | `https-argocd` (HTTPS, terminate) | HTTPRoute `argo/argocd` | `argocd-server:80` |
| `tekton.<suffix>` | `https-tekton` (HTTPS, terminate) | HTTPRoute `home-sso-auth/tekton-dashboard` | `tekton-oauth2-proxy:80` |
| `pipelines-as-code.<suffix>` | `https-pac` (HTTPS, terminate) | HTTPRoute `pipelines-as-code/pipelines-as-code` | `pipelines-as-code-controller:8080` |
| `login.<suffix>` | `tls-login` (TLS, passthrough) | TLSRoute `pinniped-system/pinniped-supervisor` | `pinniped-supervisor:443` |
| `kubectl.<suffix>` | `tls-kubectl` (TLS, passthrough) | TLSRoute `pinniped-system/pinniped-impersonation` | `pinniped-concierge-impersonation-proxy-cluster-ip:443` |
| three UI hosts (HTTP :80) | `http` | HTTPRoute `istio-ingress/redirect-to-https` | 301 to HTTPS |

Details:

- One explicit multi-SAN `Certificate` `internal-tls` (`home-issuer`) serves all
  three terminated listeners. The cert-manager Gateway shim is **not** used.
- Pinniped keeps serving its own certificates through TLS passthrough, exactly
  like the old `ssl-passthrough` Ingress.
- `spec.infrastructure.labels: istio.io/dataplane-mode: ambient` enrols the
  generated gateway pod. A namespace label alone does not.
- `allowedRoutes.namespaces.from: Selector` with label
  `istio.io/edge-route: enabled`. The label is applied to `istio-ingress`,
  `argo`, `home-sso-auth`, `pipelines-as-code` and `pinniped-system` by the
  helmfile presync hook (`kustomizations/edge-route-namespaces/`). Never label
  tenant namespaces.

The address and host suffix are environment values; the chart defaults are
TEST-NET-1 documentation values:

| Variable | Meaning |
|---|---|
| `EDGE_LB_IP` | MetalLB address for the Gateway (the address ingress-nginx used) |
| `EDGE_HOST_SUFFIX` | `sslip.io` suffix, i.e. `<lb-ip>.sslip.io` |

```sh
EDGE_LB_IP=<lb-ip> \
EDGE_HOST_SUFFIX=<lb-ip>.sslip.io \
helmfile -f helmfile-edge.yaml.gotmpl apply
```

On a first install, wait for the address, the certificate and every route:

```sh
kubectl wait --for=condition=Programmed gtw/istio-internal -n istio-ingress
kubectl wait --for=condition=Ready certificate/internal-tls -n istio-ingress
kubectl get httproute,tlsroute -A
```

Every route must report `Accepted=True` and `ResolvedRefs=True`. `istioctl
analyze -A` must show no Errors or Warnings; `IST0102` (no sidecar injection on
ambient namespaces) and `IST0118` (port naming on unrelated services) are known
Info-level false positives.

Verify the hosts (use `--noproxy '*'` when an HTTP proxy is configured, or the
proxy ignores `--resolve`):

```sh
kubectl -n cert-manager get secret home-root-ca \
  -o jsonpath='{.data.tls\.crt}' | base64 -d > home-ca.crt

curl --noproxy '*' -sS --cacert home-ca.crt -o /dev/null -w '%{http_code}\n' \
  --resolve argocd.<lb-ip>.sslip.io:443:<lb-ip> https://argocd.<lb-ip>.sslip.io/
# 200; /auth/login returns 303 to the Cognito hosted UI
curl --noproxy '*' -sS -o /dev/null -w '%{http_code}\n' --cacert home-ca.crt \
  --resolve pipelines-as-code.<lb-ip>.sslip.io:443:<lb-ip> \
  https://pipelines-as-code.<lb-ip>.sslip.io/
# login discovery 200, kubectl /version 401 (impersonation proxy; use -k)
```

### Scheduling risk (accepted)

The deleted ingress-nginx controller had a `nodeSelector` on the general
workers. The Gateway API `Gateway` does **not**: Istio generates the
Deployment with no node affinity, so the single gateway replica can land on
the CI node (Dell, `node-role.kubernetes.io/ci`, `PreferNoSchedule`). There it
competes with PipelineRuns for CPU and disk, and CI bursts can delay edge
traffic.

This is a known, accepted risk in iteration 1. Nothing in the chart pins it:

- `spec.infrastructure.labels/annotations` only label the generated
  Deployment/Service; they cannot express a `nodeSelector`.
- The supported pinning path is a `GatewayClass.spec.parametersRef` pointing
  at a ConfigMap with gateway Deployment values (`nodeSelector`/`affinity`).
  That means owning the `istio` GatewayClass (currently shipped by the
  `istio-base` chart) and re-applying it after every Istio upgrade, so it is
  deliberately not implemented here.

If it becomes a problem: add the `parametersRef` ConfigMap, or give the
gateway a hard node affinity through the same mechanism, and record the
change in the service-networking ADR first.

The ticket #11 temporary gateway (`istio-temp`, `charts/edge-temp-gateway`)
was retired after this gateway was verified. Its proof remains in git history;
do not reintroduce a second gateway on the production address.

## Sandbox

`ambient-sandbox` (ambient) runs an unprivileged nginx server and a curl
client; `PeerAuthentication` is `STRICT` (the namespace's PodSecurity is
`baseline`, not privileged). `ambient-sandbox-nomesh` has the same curl client
but no ambient label, so its plaintext request is rejected by ztunnel. Both
namespaces are scratch:

```sh
helmfile -l stage=sandbox destroy
```

That destroy is **not permanent**: both releases are part of `helmfile.yaml`,
so the next `helmfile apply` recreates them. To retire the proof for good,
delete the two `stage: "sandbox"` releases from `helmfile.yaml`.

The `ambient-probe-policy` release is not scratch: it must stay as long as
ambient namespaces exist under a default-deny Cilium policy.

## Cleanup

Routine teardown (keep the Gateway API CRDs — every Gateway, HTTPRoute and
TLSRoute in the cluster depends on them):

```sh
helmfile -l 'stage!=crds' destroy
```

A plain `helmfile destroy` also uninstalls `gateway-api-crds`, which deletes
the Gateway API CRDs cluster-wide and cascades every Gateway/HTTPRoute/TLSRoute
with them. Only do that when you are abandoning Istio altogether, and expect to
recreate all gateway objects afterwards.

`home-ingress-nginx` no longer exists in this repo; rollback reinstalls it from
the pre-cutover commit (see `developer/docs/operations/istio-ambient-cutover.md`).

The edge Gateway is removed separately (and releases the address):

```sh
helmfile -f helmfile-edge.yaml.gotmpl destroy
```

Destroy the edge **before** `home-istio` itself: the generated gateway
Deployment/Service and its routes disappear with the edge release, and the
`istio-ingress` Gateway API CRD objects must exist while that happens.
