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

- `../home-cluster-1-config/` — kubeconfig + talosconfig only (sibling directory, ignored by git). Machine configs and the cluster CA/secrets are now managed by `platform/infra/terraform-home-talos/`; the CA lives in that unit's S3 Terraform state.
- Component-local `secrets/` — each is gitignored; only `*.example.yaml` is committed.
- The restricted repo — `home-cluster-internal`.

## Adding something

One rule keeps this navigable: a new feature is a directory **inside**
`platform/cluster/`, `platform/cicd/`, or `developer/`, plus one row in the
inventory above. Nothing gets added at the top level.

If a change is about *running* the cluster it goes in `platform/`. If a
developer needs it to ship their app, it goes in `developer/`. If it must not
be public, it goes in `home-cluster-internal`.
