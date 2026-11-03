<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-01 -->

# AGENTS.md (home-cilium)

Day-0 base component: Cilium is the cluster CNI (`kube-system`). It must be installed before MetalLB — nothing has pod networking until the CNI runs. Flannel is disabled in the Talos machine config.

## Overview
Overview of this component — see `README.md` for prose.

## Setup (deploy & teardown)

| Task | Command |
|------|---------|
| Prerequisite | Patch every Talos node with `platform/node/patches/cni-cilium.yaml` (`cluster.network.cni.name: none`) |
| Apply | `helmfile apply` (this dir) |
| Confirm | `kubectl -n kube-system rollout status ds/cilium`; `kubectl get crd ciliumnetworkpolicies.cilium.io` |
| Verify | `./verify-isolation.sh` from the workspace root |
| Cleanup | `helmfile destroy` — then follow the rollback (node reboot, Flannel patch) in `README.md` |

## Commands
Full command table lives in the Setup section; offline validation: `helm lint ./charts/upstream/cilium-1.20.4.tgz -f values/cilium-values.yaml` and `helm template`, via the alpine/helm container (see workspace root AGENTS.md).

## Key files

| File | Notes |
|------|-------|
| `charts/upstream/cilium-1.20.4.tgz` | pinned; refresh: `helm pull cilium/cilium --version <latest> -d charts/upstream` |
| `values/cilium-values.yaml` | `kubeProxyReplacement: false`, `ipam.mode: kubernetes`, VXLAN, Talos capability set, cgroup automount off, Hubble off, LB IPAM off (MetalLB owns LB), `cni.exclusive: false` (Istio CNI chaining) |
| `verify-isolation.sh` | the ticket-#2 acceptance test; fails on Flannel by design |

## Conventions & rules

- Keep `kubeProxyReplacement: false` and `ipam.mode: kubernetes` (ADR decision). Do not flip to kube-proxy replacement in a policy change.
- Keep `cni.exclusive: false` while Istio ambient (`home-istio`) is installed: the Istio CNI plugin lives in the same `/etc/cni/net.d` chain. `true` deletes it and breaks ambient redirection.
- Keep `bpf.masquerade: false`: Istio ambient does not support Cilium BPF masquerading for kubelet health checks.
- The Talos capability set must keep `SYS_MODULE` dropped; Talos does not allow workloads to load kernel modules.
- `cgroup.autoMount.enabled` must stay `false` with `hostRoot: /sys/fs/cgroup`; Talos already mounts cgroupv2.
- `kube-flannel` must be deleted only after `ds/cilium` and `deploy/cilium-operator` are Ready — see the rollout order in `README.md`.
- Cilium `1.20.x` does not ship CRDs in the Helm chart; the operator registers them. Wait for `ciliumnetworkpolicies.cilium.io` before tenant policies.
- Never edit the vendored `.tgz`.
- Cross-repo rules: workspace root `AGENTS.md`.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- The verification script uses `restricted` PSA namespaces and drops all capabilities; keep it that way.
- `<ingress-ip>`, `<your-github-org>`, `<region>`, `<user-pool-id>` are TODOs, not values to invent.

## Checklist
- [ ] helmfile chart paths resolve (no dangling `./charts/...`)
- [ ] `helm lint` and `helm template` pass with `values/cilium-values.yaml`
- [ ] The Talos CNI patch is referenced, not duplicated, in docs
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value
- [ ] `verify-isolation.sh` passes `bash -n` and shellcheck
