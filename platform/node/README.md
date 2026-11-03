# Talos (platform/node)

Tooling to install and manage the `home-cluster-1` Talos cluster.

Cluster devices from physical top-down location:
| Node | Role | IP |
|------|------|----|
| home-cluster-1 (talos-pil-rbl) | worker — Lenovo | 192.168.1.106 |
| home-cluster-1 (talos-ioc-nnr) | worker — Lenovo | 192.168.1.109 |
| home-cluster-1 (talos-f1k-nu0) | control plane | 192.168.1.107 |
| home-cluster-1 (talos-3qb-723) | worker — CI node (Tekton PipelineRuns) | 192.168.1.108 |

Versions: Talos `v1.x.x`, Kubernetes `v1.x.x`.

## Prerequisites

- `talosctl` installed: see [install-talos-linux.md](./install-talos-linux.md#talosctl)
- How to connect: https://github.com/stijnklomp-talos-linux-home-setup/home-cluster (instructions in `resources/instructions/kubernetes.md`)
- New hardware: see [bios-power-settings.md](./bios-power-settings.md) (power + AC-recovery BIOS checklist)

## One-time install

1. See [install-talos-linux.md](./install-talos-linux.md#cluster-setup). This flashes the Talos ISO and bootstraps the control plane.

## CNI (Cilium)

The cluster uses Cilium, not Talos' default Flannel. Every node's machine config
must have `cluster.network.cni.name: none`; the committed patch is
[`patches/cni-cilium.yaml`](./patches/cni-cilium.yaml).

- **Fresh cluster:** `talosctl gen config home-cluster-1 https://192.168.1.107:6443 --config-patch @patches/cni-cilium.yaml`
- **Existing node:** `talosctl --talosconfig=./talosconfig patch machineconfig --nodes <node-ip> --patch @patches/cni-cilium.yaml` (dry-run first, repeat per node)
- **New worker:** `add-worker-node.yaml` applies the same patch automatically (`CNI_PATCH_FILE` overrides it).

Cilium itself is installed from
`platform/cluster/home-cilium/` (`helmfile apply`).
That README also holds the rollout order (Cilium first, then remove the residual
Flannel DaemonSet, then restart workloads) and the rollback to Flannel.

`platform/node/patches/cni-flannel.yaml` is the rollback patch.

## Rootless build prerequisites

Tenant pipeline builds run under Pod Security `restricted`, so the image build
uses rootless Buildah. Two node-level settings make that possible:

| Patch | Purpose | Apply mode |
|---|---|---|
| `patches/user-namespaces.yaml` | `user.max_user_namespaces: "15000"` (Talos default is 0) | immediate (no reboot) |
| `patches/buildah-seccomp.yaml` | installs `/var/lib/kubelet/seccomp/profiles/buildah-userns.json`, the Localhost seccomp profile that allows the namespace/mount syscalls rootless Buildah needs | `--mode=staged`, applied at the next reboot |

```sh
talosctl --talosconfig=./talosconfig patch machineconfig --nodes <node-ip> \
  --patch @patches/user-namespaces.yaml
talosctl --talosconfig=./talosconfig patch machineconfig --nodes <node-ip> \
  --mode=staged --patch @patches/buildah-seccomp.yaml
```

A node without the seccomp profile fails builds at
`unshare(CLONE_NEWUSER): operation not permitted`.

## Add new worker node

1. See [install-talos-linux.md](./install-talos-linux.md#add-worker-node-to-cluster). This joins the worker node(s).
2. After the node joins, designate its role (idempotent — also re-run after **rebuilding** a node, i.e. a fresh Talos install; not needed for plain reboots since labels/taints live in etcd):

```sh
./designate-node-roles.yaml # requires kubectl
```

- New **general worker**: gets the `node-role.kubernetes.io/worker` label — required by the MetalLB speaker and Tekton controllers (both `nodeSelector` on it). The Istio edge gateway (since 2026-11) has no node selector and may run on any node.
- New **CI node** (Tekton-preferred): gets the `node-role.kubernetes.io/ci` label + `PreferNoSchedule` taint — TaskRuns prefer it (soft nodeAffinity in the TektonConfig default pod template) and spill to the general workers when it is full; other workloads avoid it unless every other node is full.
- The script's node lists are in the cluster-facts table at the top — update them if a node's IP changes.

## Spin cluster UP

1. Power on the machines.
2. The control plane boots and the workers join automatically.
3. Wait for the workers to be Ready and uncordon them: (requires kubectl)

```sh
./startup-worker-nodes.yaml
```

## Spin cluster DOWN

1. Spin entire cluster down (cordon + drain all worker nodes): (requires kubectl)

```sh
./spindown-worker-nodes.sh
```

## Cluster capacity report

Read-only CPU/RAM/storage snapshot of all nodes (build once, then run):

```sh
docker build -t home-cluster-capacity cluster-capacity
docker run --rm -it -e WATCH=5 -v "$PWD/../home-cluster-1-config:/cfg:ro" home-cluster-capacity
```

## Heat and fans report

Read-only temperatures (°C) and fan RPM per node (build once, then run):

```sh
docker build -t home-cluster-heat-fans heat-fans
docker run --rm -it -e WATCH=5 -v "$PWD/../home-cluster-1-config:/cfg:ro" home-cluster-heat-fans
```

`VIEW=complex` shows every sensor instead of the hottest per component:

```sh
docker run --rm -it -e VIEW=complex -v "$PWD/../home-cluster-1-config:/cfg:ro" home-cluster-heat-fans
```

## Upgrade Talos Linux

Check the latest stable Talos version:

```sh
curl -s https://api.github.com/repos/siderolabs/talos/releases/latest | jq -r .tag_name
```

1. **Upgrade `talosctl` on your machine first** — it must be at least the version you're upgrading to (check with: `talosctl version --client`):

```sh
VERSION=v1.13.9 ./install-talosctl.sh
```

2. Run the upgrade script — it handles everything: etcd snapshot, control plane upgrade, health check, then each worker (cordon → drain → upgrade → uncordon):

```sh
./upgrade-talos.yaml v1.13.9
```

- Expected: a few minutes of cluster-API downtime while the single control plane reboots; workloads on workers keep running.
- Rollback (emergency only, if a node fails to boot): `talosctl rollback -n <node-ip>`.

## Upgrade Kubernetes

Separate step, run after the Talos upgrade. Check the latest stable Kubernetes version:

```sh
curl -s https://api.github.com/repos/kubernetes/kubernetes/releases/latest | jq -r .tag_name
```

Check which Kubernetes versions **your current Talos release supports**, what version the cluster is on now, and the latest patch of the highest supported minor:

```sh
./upgrade-k8s.yaml check
```

> The latest stable K8s may be newer than what your Talos version supports, and upgrades are one minor version at a time (1.35 → 1.36 → 1.37). Check the Talos/K8s support matrix before choosing.

A minor-only version auto-resolves to its latest patch:

```sh
./upgrade-k8s.yaml <K8S_VERSION> <CLUSTER_CONFIG_DIR> [ROOT_HCL]
# e.g. ./upgrade-k8s.yaml v1.36 cluster-config/ ./root.hcl # (resolves to latest patch version e.g. v1.36.4)
```

- `<CLUSTER_CONFIG_DIR>` — **required** (the script errors if omitted): a directory holding `controlplane.yaml`/`worker.yaml`. Those files are no longer stored: generate them into a temp dir from Terraform first (`terragrunt output -raw machine_config_controlplane`, `terragrunt output -json machine_config_workers`). After the upgrade the script syncs their component image versions (kube-apiserver/controller-manager/scheduler/kube-proxy/kubelet) to the new version.
- `[ROOT_HCL]` — optional: pass the `root.hcl` path (shown above as `./root.hcl`) to also sync its `k8s_version`. Omit to skip.

Runs a dry run first, then the real upgrade against the control plane.
