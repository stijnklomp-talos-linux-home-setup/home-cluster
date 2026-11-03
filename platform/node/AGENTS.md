<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-10-24 | Last verified: 2026-09-23 -->

# AGENTS.md (platform/node)

Tooling to install and manage the `home-cluster-1` Talos Linux cluster — the entry point for node bring-up, worker joins, upgrades, and cluster spin up/down. The scripts may be run against the live cluster when the user asks.

## Default cluster (connecting)

**Unless the user specifies otherwise, the default cluster for `talosctl` and `kubectl` is always `home-cluster-1-config/`** (workspace root — `talosconfig` + `kubeconfig`, gitignored). Run both CLIs via Docker, mounting the config dir read-only:

```sh
docker run --rm -v "$PWD/home-cluster-1-config:/cfg:ro" \
  ghcr.io/siderolabs/talosctl:v1.13.9 \
  --talosconfig /cfg/talosconfig <args>

docker run --rm -v "$PWD/home-cluster-1-config:/cfg:ro" \
  registry.k8s.io/kubectl:v1.36.4 \
  --kubeconfig /cfg/kubeconfig <args>
```

`$PWD` assumes the workspace root; use the absolute path otherwise. Scripts default to `./talosconfig`/`./worker.yaml`, but the plaintext machine configs no longer exist — they are managed by `platform/infra/terraform-home-talos/`.

To feed a script that needs them, export the generated files first:

```sh
cd platform/infra/terraform-home-talos/prod
terragrunt output -raw machine_config_controlplane > /tmp/controlplane.yaml
terragrunt output -json machine_config_workers | jq -r '.["192.168.1.108"]' > /tmp/worker.yaml
```

then point `WORKER_CONFIG` / the script's config-dir argument at `/tmp`.

**The cluster is LIVE and always in use (production-like) — never assume it is idle.** Read-only commands (`get`, `list`, `describe`, `logs`, `events`, `version`, `health`) are always allowed. **Every other command — any write/mutating action** (`apply-config`, `upgrade`, `reset`, `remove-member`, `drain`, `cordon`, `uncordon`, `reboot`, `shutdown`, `kubectl apply/delete/scale/exec`, etc.) **must be confirmed with the user first**, even if it looks harmless.

## Cluster facts (fixed, do not invent)

| Node | Role | IP |
|------|------|----|
| home-cluster-1 (talos-f1k-nu0) | control plane | 192.168.1.107 |
| home-cluster-1 (talos-3qb-723) | worker — **CI node** (Tekton PipelineRuns) | 192.168.1.108 |
| home-cluster-1 (talos-pil-rbl) | worker — Lenovo | 192.168.1.106 |
| home-cluster-1 (talos-ioc-nnr) | worker — Lenovo | 192.168.1.109 |

- Versions: Talos `v1.13.9`, Kubernetes `v1.36.4`. Update only after verifying current stable upstream.
- CNI: **Cilium** (`kube-proxy` retained), not Talos' default Flannel. Every node's machine config must carry `cluster.network.cni.name: none` from `patches/cni-cilium.yaml`; a node built without it runs Flannel alongside Cilium. Rollout/rollback: `platform/cluster/home-cilium/README.md`.
- Node roles: set via `designate-node-roles.yaml` (control plane stays kube-system only; CI node is tainted `node-role.kubernetes.io/ci:PreferNoSchedule` — Tekton TaskRuns prefer it via soft nodeAffinity, everything else avoids it unless no other node fits).
- Connect flow (kubeconfig / TALOSCONFIG): `home-cluster-internal/wiki/resources/instructions/kubernetes.md` — not in this repo.

## Files

| File | What it is |
|------|-----------|
| `README.md` | Main usage guide: install, add worker, spin up/down. |
| `install-talos-linux.md` | Versioned install steps: flash ISO, bootstrap, join worker. |
| `install-talosctl.sh` | Downloads + checksum-validates talosctl; replaces an older installed version automatically (`VERSION=v1.13.9 ./install-talosctl.sh`). |
| `add-worker-node.yaml` | **Bash script** (despite `.yaml`): lists disks, auto-detects the internal (sata/nvme) disk, applies `worker.yaml` with `install.disk` patched per node, joins it. |
| `upgrade-talos.yaml` | **Bash script**: upgrade all nodes to a target Talos version — etcd snapshot, CP first (single CP = brief API downtime), then workers one at a time with drain/uncordon. Workers use the Image Factory schematic installer (`WORKER_INSTALLER_IMAGE`) — it carries the iscsi-tools extension Longhorn needs plus the low-power kernel args; the control plane uses its own schematic (`CONTROL_PLANE_INSTALLER_IMAGE`). Env: `TALOSCONFIG`, `CONTROL_PLANE_IP`, `WORKER_IPS`, `WORKER_INSTALLER_IMAGE`, `CONTROL_PLANE_INSTALLER_IMAGE` (defaults: kubectl discovery + schematics in `schematics/`). |
| `upgrade-k8s.yaml` | **Bash script**: `check` mode prints supported K8s versions, current cluster kubelet version, and the latest patch of the max minor; upgrades via `talosctl upgrade-k8s` (dry-run first). `<CLUSTER_CONFIG_DIR>` arg is **required** (syncs stored `controlplane.yaml`/`worker.yaml` component images afterwards); optional `[ROOT_HCL]` arg syncs `k8s_version` too. |
| `startup-worker-nodes.yaml` | **Bash script**: wait for Ready + uncordon workers (needs kubectl). |
| `designate-node-roles.yaml` | **Bash script**: label/taint nodes by role (CI node = Dell .108, workers = Lenovos .106/.109); idempotent, re-run after node re-joins. |
| `drain-worker-nodes.yaml` | **Bash script**: cordon + drain workers before shutdown (needs kubectl). All workers by default; pass worker IPs as args (or `WORKER_IPS`) to drain a subset; refuses the control-plane IP. |
| `cluster-capacity/` | One tool per Dockerfile: `Dockerfile` (copies `kubectl` from `registry.k8s.io/kubectl:v1.36.4` into `python:3.13-slim`, installs `rich`), `cluster-capacity.py`, and a whitelist `.dockerignore`. Read-only report of per-node actual/requested/available CPU, RAM, and storage: actual usage from the kubelet `/stats/summary` proxy (no metrics-server needed), pod requests/limits from pod specs, Longhorn pool/volumes from the `longhorn-system` CRDs. Width-adaptive output; `WATCH=<s>` refreshes on a TTY; kubeconfig mounted read-only at runtime (`ENV KUBECONFIG=/cfg/kubeconfig`) — never baked in; `TZ` default `Europe/London`. |
| `heat-fans/` | Same pattern (copies `talosctl` from `ghcr.io/siderolabs/talosctl:v1.13.9` into `python:3.13-slim` + `rich`, whitelist `.dockerignore`). Read-only heat/fan report: discovers nodes via `talosctl get members`, reads `/sys/class/hwmon` per node (`coretemp`/`acpitz`/`nct6686`/`drivetemp`/`nvme`) with a small thread pool; unconnected thermistors (chip max, no critical) are hidden. Nodes shown by role (`controller`, `worker 1`, …; node name appended at ≥130 cols). `VIEW=simple` (default) = hottest temp per component (CPU/GPU/RAM/Storage/Board), `VIEW=complex` = every sensor. Temperature bars show the value on a 0–100 °C scale; number and bar use a smooth blue→green→yellow→red gradient over 30–90 °C (every degree shifts the color, monotonic with temperature), so overheating stands out. The image sets `TERM=xterm-256color` + `COLORTERM=truecolor` so Docker does not downgrade the gradient to 16 colors (override with `-e COLORTERM=` on old terminals). Width-adaptive; `WATCH=<s>`; talosconfig mounted read-only at runtime (`ENV TALOSCONFIG=/cfg/talosconfig`) — never baked in; `TZ` default `Europe/London`. |
| `schematics/` | Image Factory schematics (source of truth for installer images): `controlplane-schematic.yaml` (stock + low-power kernel args), `worker-schematic.yaml` (iscsi-tools + same args). Kernel args are baked into the UKI at image build time — re-mint + upgrade nodes to change them. |
| `patches/cni-cilium.yaml` | Talos v1.13 machine-config patch: `cluster.network.cni.name: none` so Cilium owns pod networking. Applied to new workers by `add-worker-node.yaml`; pass it to `talosctl gen config` for a rebuild and to `talosctl patch machineconfig` on existing nodes. |
| `patches/cni-flannel.yaml` | Rollback patch: `cluster.network.cni.name: flannel`. Only with the full rollback procedure in the `home-cilium` README (destroy Cilium, patch, reboot, restart workloads). |
| `patches/user-namespaces.yaml` | `machine.sysctls.user.max_user_namespaces: "15000"`. Required by the rootless Buildah task (`restricted` PSA); Talos defaults to 0. Applies at runtime with `--mode=no-reboot` and persists at boot. |
| `patches/buildah-seccomp.yaml` | `machine.files` drop-in of `/var/lib/kubelet/seccomp/profiles/buildah-userns.json`, the Localhost seccomp profile the rootless build step uses. `machine.files` cannot be applied in immediate mode: stage it (`--mode=staged`) and reboot at the next maintenance window. |

## Rules

| Rule | Detail |
|------|--------|
| `.yaml` = bash | All `*.yaml` scripts are executable shell scripts with `#!/usr/bin/env bash` — run them directly, never `kubectl apply`. |
| Secrets stay local | `talosconfig` and `worker.yaml` live in this dir but are **not committed** (node join secrets). Never add them, or real IPs of future nodes, to git. |
| Etcd snapshots never commit | `backups/*.db` = a full etcd dump: **every** Kubernetes Secret + Talos machine CA. They are local disaster-recovery only (`.gitignore` blocks `backups/`). This repo is PUBLIC — a pushed snapshot leaks all cluster credentials and forces a full rotation. |
| Default cluster | `talosctl`/`kubectl` always target `home-cluster-1-config/` (workspace root) unless the user says otherwise — see "Default cluster" above. The cluster is live/production: read-only commands are always allowed; **every non-read command must be user-approved first**. |
| Static IP first | A new worker needs a router DHCP reservation **before** joining (`install-talos-linux.md`). Never fabricate a node IP. |
| Verify disk before apply | `add-worker-node.yaml` prints the node's disks, auto-detects the internal disk (`transport: sata`/`nvme`, never `usb`/`loop0`) and applies it via `apply-config --config-patch` — `worker.yaml` itself stays generic. Override: `WORKER_DISK=/dev/disk/by-id/...`. |
| Upgrade order | `upgrade-talos.yaml`: control plane → workers one at a time; never parallel upgrades (single CP, no quorum). `upgrade-k8s.yaml` only after Talos is at a supporting version; sync stored configs + `root.hcl` after it. |
| Env overrides | No env vars are required — scripts default to the cluster facts above (CP `192.168.1.107`, workers auto-discovered via kubectl). Optional overrides still honored: `WORKER_IP`, `WORKER_CONFIG`, `TALOSCONFIG`, `WORKER_DISK`, `WORKER_IPS`, `CONTROL_PLANE_IP`, `SNAPSHOT_RETENTION`, `WORKER_INSTALLER_IMAGE`. |
| Worker installer image | Workers run an Image Factory schematic installer (schematic `8e594bc9...` = `siderolabs/iscsi-tools` + `pcie_aspm` kernel args, see `schematics/`) — the plain `ghcr.io/siderolabs/installer` DROPS the extension on upgrade and Longhorn breaks. Never upgrade a worker with the plain installer. |
| User namespaces for rootless builds | `user.max_user_namespaces=15000` and the `buildah-userns` seccomp profile must exist on **every** node that can run a tenant PipelineRun. Both are in `patches/`; the file patch needs a reboot to take effect. |
| CNI patch on every node | `cluster.network.cni.name: none` must be present in the machine config of **every** node (control plane and workers). `add-worker-node.yaml` applies `patches/cni-cilium.yaml`; a manually joined node without it reintroduces Flannel. |
| Docs sync | Keep `README.md` ↔ `install-talos-linux.md` consistent when steps change (script names, env vars, versions). |

## Commands (verify paths before claiming done)

| Task | Command |
|------|---------|
| Connect to cluster | `docker run --rm -v $PWD/home-cluster-1-config:/cfg:ro ghcr.io/siderolabs/talosctl:v1.13.9 --talosconfig /cfg/talosconfig <args>` (kubectl: `registry.k8s.io/kubectl:v1.36.4 --kubeconfig /cfg/kubeconfig <args>`) |
| Install talosctl | `VERSION=v1.13.9 ./install-talosctl.sh` |
| Patch CNI to Cilium (every node) | `talosctl --talosconfig=./talosconfig patch machineconfig --nodes <NODE_IP> --patch @patches/cni-cilium.yaml` (dry-run first; control plane first) |
| Add worker node | `./add-worker-node.yaml <NODE_IP>` |
| Upgrade Talos (all nodes) | `./upgrade-talos.yaml <TALOS_VERSION>` |
| Upgrade Kubernetes | `./upgrade-k8s.yaml check` then `./upgrade-k8s.yaml <K8S_VERSION> <CLUSTER_CONFIG_DIR> [ROOT_HCL]` |
| Spin cluster up (workers) | `./startup-worker-nodes.yaml` |
| Designate node roles | `./designate-node-roles.yaml` (after adding/rebuilding a worker) |
| Spin cluster down (workers) | `./drain-worker-nodes.yaml` then shutdown each worker — README shows a manual IP list or kubectl auto-discovery |
| Spin cluster down (control plane, last) | `talosctl shutdown -n $CONTROL_PLANE_IP` |
| Cluster capacity report (read-only) | From `platform/node/`: `docker build -t home-cluster-capacity cluster-capacity && docker run --rm -v "$PWD/../home-cluster-1-config:/cfg:ro" home-cluster-capacity` |
| Heat and fans report (read-only) | From `platform/node/`: `docker build -t home-cluster-heat-fans heat-fans && docker run --rm -v "$PWD/../home-cluster-1-config:/cfg:ro" home-cluster-heat-fans` |

## Checklist (before finishing any change)

- [ ] No `talosconfig`/`worker.yaml`/node secrets staged
- [ ] Script usage and env vars match README
- [ ] IPs/versions match the cluster-facts table above; no fabricated values