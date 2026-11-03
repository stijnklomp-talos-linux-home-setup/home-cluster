# Node role grouping: controller / CI / general workers

## Status

Accepted (2026-09)

## Context

The cluster grew from 2 nodes (CP + 1 worker) to 4 (CP + 3 workers). Everything used to run wherever it landed; the control plane was also a scheduling target for ingress and Tekton components. With dedicated hardware available, workloads should map to node groups so CI runs don't fight general workloads.

## Decision

Three node roles, set by `platform/node/designate-node-roles.yaml` (labels + taints):

| Role | Nodes | Taint | Runs |
|---|---|---|---|
| control-plane | HP EliteDesk (192.168.1.107) | `node-role.kubernetes.io/control-plane:NoSchedule` (Talos default) | kube-system only |
| ci | Dell (192.168.1.108) | `node-role.kubernetes.io/ci:PreferNoSchedule` | Tekton TaskRuns/PipelineRuns **prefer** it (soft nodeAffinity, weight 100, in TektonConfig default-pod-template); spills to workers when full; other workloads avoid it unless every other node is full |
| worker | 2× Lenovo (192.168.1.106/.109) | none | everything else (Pinniped, PaC, Verdaccio, ArgoCD). The Istio edge Gateway (since 2026-11) has no node affinity and may also run on the CI node — see `operations/istio-ambient-cutover.md` |

Longhorn tolerates the CI taint so TaskRun workspace volumes can attach on the Dell; it does not tolerate the control-plane taint, so the CP disk is not part of storage.

## Rationale

- The Dell (dedicated CI node) keeps CPU/disk headroom deterministic for PipelineRuns; general workloads automatically flow to the Lenovos because the CP is NoSchedule-tainted and the CI node is PreferNoSchedule-tainted.
- The CP stays kube-system only, reducing blast radius on the single control plane (no quorum — accepted trade-off).
- Soft taint + soft affinity (not hard pinning): TaskRuns spill to the Lenovos when the Dell fills up, and non-Tekton workloads can still land on the Dell as a last resort when every other node is full — "prefer, not pin". (Changed 2026-09: was NoSchedule + nodeSelector hard-pinning.)

## Consequences

Easier: predictable placement, CI isolation, CP stability, CI survives a busy Dell (spill). Harder: no hard isolation anymore — during CI bursts the Dell may temporarily host non-Tekton pods; if the Dell is down, CI is degraded (TaskRuns spill to workers); a new worker must be re-designated after joining (`designate-node-roles.yaml` is idempotent).