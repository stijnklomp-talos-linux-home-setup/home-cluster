# home-cluster

Everything for the `home-cluster-1` home lab in one repo: the code that makes
the cluster run, and the contract you need to deploy an app onto it.

Two zones, split by who needs them:

| Zone | Question it answers | Audience |
|------|--------------------|----------|
| [`platform/`](platform/README.md) | *How does the cluster run?* | whoever operates the cluster |
| [`developer/`](developer/README.md) | *How do I deploy my app?* | anyone building on the cluster |

Restricted material is **not** here. It lives in a separate private repo,
`home-cluster-internal`, cloned next to this one. Nothing security-sensitive is
duplicated into this repo — see [Where credentials live](#where-credentials-live).

## Start here

- **I want to deploy an app** → [`developer/README.md`](developer/README.md) — four steps, with a working example to copy.
- **I want to understand the design** → [`developer/docs/architecture-decision-log/`](developer/docs/architecture-decision-log/README.md) — the ADRs, in order.
- **I want to run or repair the cluster** → [`platform/README.md`](platform/README.md) — components, install order, teardown order.
- **I need the restricted runbooks** → clone `home-cluster-internal` (access is separate, by design).
- **I'm setting up a new machine** → [Set up a machine](#set-up-a-machine) — create the sibling config dir and verify access.

## Set up a machine

This repo expects a sibling directory, `../home-cluster-1-config/`, outside the
working tree. It holds the live client configs and is gitignored — never commit
or push it.

| Requirement | Value |
|---|---|
| Docker | every cluster CLI runs in a container |
| terragrunt | on the host; the infra units invoke it directly |
| AWS CLI + SSO | profile `testing`, account `220402652377`, region `eu-west-1` |
| S3 state bucket | `home-terraform-state-220402652377` (created by `platform/infra/terraform-remote-state/`) |

### You already have cluster access

1. Create the directory with tight permissions:

   ```sh
   mkdir -p ../home-cluster-1-config
   chmod 700 ../home-cluster-1-config
   ```

2. Add `kubeconfig` and `talosconfig` (mode 600). Either copy them from the
   cluster owner, or regenerate them from Terraform state — no cluster access
   is needed for that, the outputs come from S3:

   ```sh
   aws sso login --profile testing
   cd platform/infra/terraform-home-talos/prod
   terragrunt init
   terragrunt output -raw talosconfig > ../../../../../home-cluster-1-config/talosconfig
   terragrunt output -raw kubeconfig  > ../../../../../home-cluster-1-config/kubeconfig
   chmod 600 ../../../../../home-cluster-1-config/talosconfig
   chmod 600 ../../../../../home-cluster-1-config/kubeconfig
   cd ../../../..
   ```

3. Verify from the repo root:

   ```sh
   docker run --rm -v "$PWD/../home-cluster-1-config:/cfg:ro" \
     registry.k8s.io/kubectl:v1.36.4 --kubeconfig /cfg/kubeconfig get nodes

   docker run --rm -v "$PWD/../home-cluster-1-config:/cfg:ro" \
     ghcr.io/siderolabs/talosctl:v1.13.9 --talosconfig /cfg/talosconfig version
   ```

4. Optional: clone `home-cluster-internal` next to this repo for the restricted
   runbooks.

Rebuilding the cluster from zero is a separate, partly manual process: see
[`platform/node/install-talos-linux.md`](platform/node/install-talos-linux.md),
[`platform/infra/terraform-remote-state/`](platform/infra/terraform-remote-state/),
and [`platform/infra/terraform-home-talos/`](platform/infra/terraform-home-talos/).

## What runs where

| Layer | Component | Path | Namespace | Applied by |
|-------|-----------|------|-----------|------------|
| Node | Talos machine config, patches, schematics | `platform/node/` | — | `talosctl` |
| Infra | Terraform: remote state, Cognito federation, Talos cluster | `platform/infra/` | — | `terragrunt` |
| Network | Cilium (CNI, network policies) | `platform/cluster/home-cilium/` | `kube-system` | helmfile |
| Network | MetalLB (LoadBalancer IPs) | `platform/cluster/home-metallb/` | `metallb-system` | helmfile |
| TLS | cert-manager (`home-root-ca`) | `platform/cluster/home-cert-manager/` | `cert-manager` | helmfile |
| Storage | Longhorn | `platform/cluster/home-longhorn/` | `longhorn-system` | helmfile |
| Mesh + edge | Istio ambient, internal Gateway | `platform/cluster/home-istio/` | `istio-system` | helmfile (two files) |
| CI | Tekton operator + Pipelines | `platform/cicd/cicd-tekton-operator/` | `tekton-operator`, `tekton-pipelines` | helmfile |
| CI | Pipelines-as-Code | `platform/cicd/cicd-tekton-pipelines-as-code/` | `pipelines-as-code` | helmfile |
| CI | Verdaccio npm cache | `platform/cicd/cicd-verdaccio/` | `verdaccio` | helmfile |
| Tenancy | Tenant registry (namespaces, quotas, PaC repos) | `platform/cicd/tenant-platform/` | one per tenant | helmfile |
| CD | ArgoCD | `platform/cicd/cicd-argocd/` | `argo` | helmfile (day 0) |
| CD | ApplicationSets + bootstrap | `platform/gitops/` | `argo` | `kubectl apply` once, then ArgoCD |
| Contract | Docs, ADRs, runbooks | `developer/docs/` | — | — |
| Contract | Reusable Tekton tasks/pipelines | `developer/pipelines/` | `tekton-pipelines` | helmfile |
| Contract | One declaration per (app, cluster) | `developer/apps/` | — | ArgoCD |
| Contract | Reference apps | `developer/examples/` | per app | ArgoCD |

## Where credentials live

Never in this repo. Live material stays outside the working tree:

- `../home-cluster-1-config/` — `kubeconfig` + `talosconfig` only. Created per [Set up a machine](#set-up-a-machine); gitignored and kept outside the tree so it can never be pushed. Machine configs and the cluster CA are now managed by `platform/infra/terraform-home-talos/`; the CA lives in that unit's S3 Terraform state.
- Component-local `secrets/` — each is gitignored; only `*.example.yaml` is committed.
- The restricted repo — `home-cluster-internal`.

## Adding something

One rule keeps this navigable: a new feature is a directory **inside**
`platform/cluster/`, `platform/cicd/`, or `developer/`, plus one row in the
inventory above. Nothing gets added at the top level.

If a change is about *running* the cluster it goes in `platform/`. If a
developer needs it to ship their app, it goes in `developer/`. If it must not
be public, it goes in `home-cluster-internal`.
