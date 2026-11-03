# Kubernetes base layer

Day-0 components of the home cluster, deployed with helmfile before ArgoCD takes over. One component per directory; each is a self-contained helmfile project.

## Components

| Directory | What it does |
|-----------|--------------|
| `home-cilium` | Cluster CNI (Cilium) — policy-capable replacement for Flannel; required before MetalLB and for tenant `CiliumNetworkPolicy` enforcement |
| `home-metallb` | Bare-metal LoadBalancer (L2) — gives services IPs from a LAN address pool (`<lb-ip-range>`, set in `home-metallb/kustomizations/metallb-config/ipaddresspool.yaml`) |
| `home-cert-manager` | TLS certs; internal CA `home-issuer` for all LAN hosts, Cloudflare DNS-01 reserved for later |
| `home-longhorn` | Distributed storage, default StorageClass `longhorn` (Tekton workspaces, plans PVC, stateful services) |
| `home-istio` | Istio 1.31.3 control plane + ambient mTLS (no sidecars), Gateway API CRDs, the production edge `Gateway` `istio-internal`, and the ambient sandbox proof |

`ingress-nginx` was removed in the ticket #12 cutover (`home-ingress-nginx`
deleted from this repo). The Istio edge serves ArgoCD, Tekton Dashboard, PaC
and Pinniped on the same MetalLB address and the same hostnames; rollback is a
`git revert` of the cutover commit plus reinstall, documented in
`developer/docs/operations/istio-ambient-cutover.md`.

## Deploy

In this order. Cilium needs the Talos machine config patched to
`cluster.network.cni.name: none` first (see `home-cilium/README.md`); Istio
needs Cilium's `cni.exclusive: false`:

```sh
# prerequisite: patch every Talos node — see home-cilium/README.md
cd home-cilium && helmfile apply
cd home-metallb && helmfile apply
cd home-cert-manager && helmfile apply
cd home-longhorn && helmfile apply
cd home-istio && helmfile apply   # first run on a fresh cluster: --skip-diff-on-install
```

The edge Gateway is a separate step because its address and host suffix are
environment values that must not be committed:

```sh
cd home-istio
EDGE_LB_IP=<lb-ip> EDGE_HOST_SUFFIX=<lb-ip>.sslip.io \
  helmfile -f helmfile-edge.yaml.gotmpl apply
```

Services are reachable at `https://<svc>.<lb-ip>.sslip.io` once the Gateway
has its MetalLB address. The `HTTPRoute`/`TLSRoute` objects are owned by the
components that own their namespaces (`home-cluster-kubernetes-cicd`,
`home-sso-auth-rbac`), not by this repo.

## Cleanup

Per directory, in reverse order. The edge Gateway is destroyed first so it
releases the address, while the Gateway API CRDs still exist:

```sh
cd home-istio
helmfile -f helmfile-edge.yaml.gotmpl destroy   # edge first: releases <lb-ip>
```

Routine teardown of the Istio component keeps the Gateway API CRDs, because
every Gateway/HTTPRoute/TLSRoute in the cluster depends on them:

```sh
helmfile -l 'stage!=crds' destroy
```

A plain `helmfile destroy` (in `home-istio` or elsewhere) also uninstalls
`gateway-api-crds` and therefore deletes the CRDs cluster-wide, cascading every
gateway object. Reserve that for abandoning Istio entirely. `home-ingress-nginx`
no longer exists here; rollback restores it from the pre-cutover commit — see
`developer/docs/operations/istio-ambient-cutover.md`.
