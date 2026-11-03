# Cilium

Cilium is the cluster CNI for `home-cluster-1` (Talos Linux `v1.13.9`, Kubernetes
`v1.36.4`). It replaces Talos' default Flannel because Flannel accepts
`NetworkPolicy` objects and ignores them: without a policy-capable CNI a tenant
pod can reach every other pod, the home LAN, the Talos API and the Kubernetes
API. The decision and its consequences are recorded in
[`developer/docs/architecture-decision-log/home/cilium-networkpolicy-adr.md`](../../../developer/docs/architecture-decision-log/home/cilium-networkpolicy-adr.md).

`kube-proxy` is retained (`kubeProxyReplacement: false`), IPAM is Kubernetes
host-scope (`ipam.mode: kubernetes`), routing is VXLAN tunnel over the existing
`10.244.0.0/16` pod subnet, Hubble is disabled, and Cilium's LB IPAM is off
because MetalLB owns LoadBalancer addresses. The values follow the official
Talos Cilium guide, including the Talos capability set and the cgroupv2 mount
handling.

## Files

| Path | What it is |
|------|------------|
| `helmfile.yaml` | Helm release `cilium` in `kube-system`, chart `1.20.4` |
| `charts/upstream/cilium-1.20.4.tgz` | vendored upstream chart — never edit in place |
| `values/cilium-values.yaml` | Talos-specific values |
| `verify-isolation.sh` | acceptance test for tenant egress isolation |

## Prerequisite: disable Flannel in the Talos machine config

Talos only stops managing Flannel when every node's machine config says so. The
committed patch is
[`platform/node/patches/cni-cilium.yaml`](../../../platform/node/patches/cni-cilium.yaml)
(Talos `v1.13.x` form; Talos `v1.14+` uses a `KubeFlannelCNIConfig` deletion
instead).

Patch every node — control plane first, then each worker. Run the dry-run
first; `--mode=no-reboot` prevents a surprise reboot. If Talos reports that a
reboot is required, apply the patch on all nodes, install Cilium (step 3), then
reboot each node before restarting workloads (step 6); rebooting before Cilium
is running would leave the node unable to start new pods.

From the workspace root:

```sh
for ip in 192.168.1.107 192.168.1.108 192.168.1.106 192.168.1.109; do
  docker run --rm \
    -v "$PWD/home-cluster-1-config:/cfg:ro" \
    -v "$PWD/platform/node:/src:ro" -w /src \
    ghcr.io/siderolabs/talosctl:v1.13.9 --talosconfig /cfg/talosconfig \
    patch machineconfig --nodes "$ip" --mode=no-reboot \
      --patch @patches/cni-cilium.yaml --dry-run
done
```

Then repeat without `--dry-run`. Talos reconciles the config immediately: it
stops managing Flannel and may remove the `kube-flannel` DaemonSet. Existing
pods keep their sandboxes and routes, but new pods cannot start until Cilium is
installed — go straight to step 3 after the loop. The API server is host
networked, so it stays reachable while the CNI is absent (unless the patch
forces a reboot).

A future `talosctl gen config` must pass the patch too, otherwise a rebuilt
cluster comes back with Flannel:

```sh
talosctl gen config home-cluster-1 https://192.168.1.107:6443 \
  --config-patch @patches/cni-cilium.yaml
```

The gitignored copies in `home-cluster-1-config/` are local files, not the
node's live machine config; `talosctl patch machineconfig` does not update
them. Patch them as well before the next `apply-config` or rebuild (do not
commit them):

```sh
for kind in controlplane worker; do
  talosctl machineconfig patch "home-cluster-1-config/${kind}.yaml" \
    --patch @platform/node/patches/cni-cilium.yaml \
    -o "/tmp/${kind}.yaml"
  mv "/tmp/${kind}.yaml" "home-cluster-1-config/${kind}.yaml"
done
```

`platform/node/add-worker-node.yaml` applies the same patch to every new
worker (`CNI_PATCH_FILE`, default `./patches/cni-cilium.yaml`).

## Rollout order

The CNI cannot be swapped without a pod-networking gap, so do this in one
maintenance window. Do **not** delete Flannel before Cilium is installed:
without any CNI, new pods (including CoreDNS) cannot start. Do **not** install
Cilium while Talos still manages Flannel either: Talos would keep reconciling
the old CNI. The order below patches Talos first, installs Cilium, then removes
the residual Flannel DaemonSet.

### 1. Pre-flight

```sh
kubectl -n kube-system get daemonset kube-flannel   # current CNI
kubectl get crd ciliumnetworkpolicies.cilium.io     # expected: NotFound
```

Stop CI (Tekton/PaC) and ArgoCD auto-sync: running PipelineRuns and app
reconciles will be disrupted by the workload restarts.

### 2. Patch Talos to `cni: none`

See [Prerequisite](#prerequisite-disable-flannel-in-the-talos-machine-config).
Do all four nodes, dry-run first.

### 3. Apply Cilium

From this directory (with `KUBECONFIG` pointing at
`home-cluster-1-config/kubeconfig`):

```sh
helmfile apply
```

Container form from the workspace root:

```sh
docker run --rm \
  -v "$PWD/platform/cluster/home-cilium:/src" \
  -v "$PWD/home-cluster-1-config:/cfg:ro" \
  -e KUBECONFIG=/cfg/kubeconfig -w /src \
  ghcr.io/helmfile/helmfile:latest -l stage=upstream apply
```

`helmDefaults.wait` returns after the `cilium` DaemonSet and operator are
Ready. The Cilium agent runs with `hostNetwork`, so it can start even while
Flannel still owns the old pods.

### 4. Confirm Cilium, then remove residual Flannel

```sh
kubectl -n kube-system rollout status daemonset/cilium --timeout=600s
kubectl -n kube-system rollout status deployment/cilium-operator --timeout=300s
kubectl -n kube-system get pods -l k8s-app=cilium -o wide
kubectl get crd ciliumnetworkpolicies.cilium.io
```

The chart (`1.20.4`) does not ship CRDs; the Cilium operator registers them
(`operator.skipCRDCreation: false`). Do not apply tenant policies until the
`CiliumNetworkPolicy` CRD exists.

Cilium took ownership of `/etc/cni/net.d` during the Flannel migration and
renamed the Flannel config to `*.cilium_bak` (`cni.exclusive: true` at the
time). That switch is now `false` because Istio ambient chains its CNI plugin
into the same directory (`home-istio`); do not flip it back while ambient is
installed. Delete the residual DaemonSet:

```sh
kubectl -n kube-system delete daemonset kube-flannel --ignore-not-found
kubectl -n kube-system get daemonset | grep -i flannel   # expected: no output
```

Optional node-side check (one node shown):

```sh
docker run --rm -v "$PWD/home-cluster-1-config:/cfg:ro" \
  ghcr.io/siderolabs/talosctl:v1.13.9 --talosconfig /cfg/talosconfig \
  ls /etc/cni/net.d -n 192.168.1.107
```

### 5. Restart workloads onto Cilium

Existing pods keep their Flannel sandboxes and IPs; only pods created after
step 3 use Cilium. Delete the pods and let their controllers recreate them.
`kubectl -n <ns> delete pods --all` is the simple form; `kubectl -n <ns>
rollout restart deployment` is the graceful alternative for Deployment-only
namespaces.

Restart CoreDNS first so new pods can resolve names:

```sh
kubectl -n kube-system delete pods -l k8s-app=kube-dns
kubectl -n kube-system wait --for=condition=Ready pod -l k8s-app=kube-dns --timeout=180s
```

Do **not** delete all pods in `kube-system`: that includes Cilium itself and
`kube-proxy`. Only the Flannel-era add-ons need a new sandbox.

### 6. Verify isolation

From the workspace root:

```sh
platform/cluster/home-cilium/verify-isolation.sh
```

Expected: a single Job prints `PASS` for DNS, Verdaccio and
`registry.npmjs.org`, and `denied` for the API service, the node API, the home
LAN, link-local metadata and the peer namespace, ending in
`RESULT: PASS - tenant isolation holds`.

## Pod restart impact

| Namespace | What is restarted | Expected impact |
|---|---|---|
| `kube-system` | CoreDNS (`k8s-app=kube-dns`), `metrics-server` | DNS errors for a few seconds; restart first and wait for Ready |
| `istio-ingress` | edge Gateway pod (`istio-internal-istio`) | edge routes (ArgoCD, Tekton, PaC, Pinniped) unavailable for roughly 30–60 s; open connections drop |
| `istio-system` | istiod, ztunnel, istio-cni-node | ambient mTLS and gateway config briefly unavailable; pods keep running |
| `metallb-system` | controller and speaker | LoadBalancer announcements blip; service IPs do not change |
| `cert-manager` | controller, webhook, cainjector | certificate issuance pauses; existing certificates keep working |
| `longhorn-system` | manager, instance-manager, UI | volume I/O pauses while engines restart — do not run during backups, rebuilds or active CI |
| `argocd` | server, repo-server, controller, redis, applicationset | UI and auto-sync pause, then resume; apps stay as they are |
| `tekton-operator`, `tekton-pipelines` | operator, controller, webhook, dashboard | running PipelineRuns/TaskRuns fail; start only when CI is idle |
| `pipelines-as-code` | controller and gosmee relay | webhook deliveries retry; no new runs during the restart |
| `verdaccio` | verdaccio | npm cache unavailable briefly; builds that need packages fail |
| app namespaces | every application pod | brief per-app outage; ArgoCD recreates what it manages |

`home-cluster` applications are not synced by this rollout; restart any app
namespace whose pods must move to Cilium (for example
`kubectl -n <app-namespace> delete pods --all`).

## Verification

`verify-isolation.sh` creates two `restricted` PSA namespaces (`t2-verify`,
`t2-verify-peer`), applies a verification copy of the tenant
`CiliumNetworkPolicy`, starts an HTTP peer in the second namespace and runs one
Job that asserts:

- **allowed:** cluster DNS, Verdaccio `:4873`, `https://registry.npmjs.org`;
- **denied:** `kubernetes.default.svc:443`, `192.168.1.107:6443`,
  the Talos API `192.168.1.107:50000`, `192.168.1.107:443`, the ingress
  LoadBalancer `192.168.1.200:443`, `169.254.169.254`, and the peer in the
  other namespace.

The script is idempotent and deletes both namespaces on exit. Use
`--keep`/`KEEP_NAMESPACES=1` to keep them for debugging, `--render` to print
the manifests without touching the cluster, and `CLUSTER_CONFIG_DIR` if
`home-cluster-1-config` is not under the current directory.

On Flannel the script fails: the `CiliumNetworkPolicy` CRD is absent, and even
if the CRD is installed the denied probes succeed because nothing enforces the
policy. That is the point of the test; it is not a substitute for the live
rollout.

## CNI chaining with Istio ambient

`cni.exclusive: false` keeps `/etc/cni/net.d` shared with other CNI plugins.
The Istio CNI node agent (`home-istio`) inserts its plugin into the same
configuration and owns the ambient redirection rules; Cilium remains the pod
network plugin. Setting it back to `true` makes Cilium rewrite the directory
and silently drop the Istio plugin, so new ambient pods get no redirection and
fail readiness. `bpf.masquerade` stays `false`: Istio ambient does not support
Cilium BPF masquerading (kubelet health checks break).

## Rollback to Flannel

This is cluster-wide and disruptive: pod networking is unavailable between
Cilium teardown and Flannel restoration. Use the same maintenance window.

1. Freeze CI and ArgoCD auto-sync.
2. Ask the Cilium agents to remove their CNI configuration and binaries when
   they stop, then uninstall the release:

   ```sh
   cd platform/cluster/home-cilium
   helm upgrade cilium ./charts/upstream/cilium-1.20.4.tgz -n kube-system \
     -f values/cilium-values.yaml --set cni.uninstall=true
   helmfile destroy
   ```

   `cni.uninstall=true` is deliberately not the committed default: it would
   remove the CNI config during every agent upgrade. It is only for teardown.
   If it is skipped, a leftover `05-cilium.conflist` in `/etc/cni/net.d` can
   still win over the Flannel config after the reboot; check with
   `talosctl ls /etc/cni/net.d` and remove it with a privileged one-shot
   DaemonSet mounting the host path before restarting workloads.

3. Restore Talos-managed Flannel on every node
   ([`platform/node/patches/cni-flannel.yaml`](../../../platform/node/patches/cni-flannel.yaml)),
   from the workspace root:

   ```sh
   for ip in 192.168.1.107 192.168.1.108 192.168.1.106 192.168.1.109; do
     docker run --rm \
       -v "$PWD/home-cluster-1-config:/cfg:ro" \
       -v "$PWD/platform/node:/src:ro" -w /src \
       ghcr.io/siderolabs/talosctl:v1.13.9 --talosconfig /cfg/talosconfig \
       patch machineconfig --nodes "$ip" --mode=no-reboot \
         --patch @patches/cni-flannel.yaml
   done
   ```

4. Reboot the nodes one at a time (control plane first) so `/etc/cni/net.d` is
   regenerated and the Cilium routes/VXLAN devices are gone:

   ```sh
   docker run --rm -v "$PWD/home-cluster-1-config:/cfg:ro" \
     ghcr.io/siderolabs/talosctl:v1.13.9 --talosconfig /cfg/talosconfig \
     reboot -n 192.168.1.107
   ```

   Talos recreates the `kube-flannel` DaemonSet from the machine config.
   Confirm with `kubectl -n kube-system get daemonset kube-flannel` and wait
   for the nodes to return to `Ready`.

5. Optional, if the CRDs must be removed (they are a dependency of the tenant
   chart, but harmless if left in place):

   ```sh
   kubectl get crd -o name | grep '\.cilium\.io$' | xargs -r kubectl delete
   ```

6. Restart workloads again (same list as step 5 of the rollout) so every pod
   gets a Flannel sandbox.

7. Verify Flannel pod connectivity (and, if step 5 was taken, that
   `ciliumnetworkpolicies.cilium.io` is gone). Tenant `CiliumNetworkPolicy`
   objects are no longer enforced once Cilium is removed, so do not leave
   tenant namespaces running on Flannel.

## Cilium CRDs are a tenant-chart dependency

The PaC tenancy platform applies `CiliumNetworkPolicy` objects in every tenant
namespace. That CRD is owned by Cilium, not by the tenant chart:

- install Cilium (and wait for the `CiliumNetworkPolicy` CRD) before applying
  tenant objects; ArgoCD/helmfile will otherwise fail on an unknown kind;
- on rollback, deleting the Cilium CRDs also deletes every tenant policy, so
  tenants must not run during the rollback window;
- the verification script checks for the CRD before it creates its scratch
  namespaces and fails with a clear message if Cilium is missing.

## Cleanup

`helmfile destroy` removes the release objects. It does not remove the Cilium
CRDs and it does not restore `/etc/cni/net.d`; those are the node reboot and
optional CRD-deletion steps in [Rollback](#rollback-to-flannel).
