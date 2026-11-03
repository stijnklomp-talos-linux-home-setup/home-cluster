# Kubernetes CI/CD layer

CI/CD stack of the home cluster, deployed with helmfile on top of the base layer. One component per directory; each is a self-contained helmfile project.

## Components

| Directory | What it does |
|-----------|--------------|
| `cicd-tekton-operator` | Tekton Operator (0.81.1), TektonConfig, standalone write-mode Dashboard (v0.72.0) |
| `cicd-tekton-pipelines-as-code` | Git-triggered CI: Pipelines-as-Code (v0.50.0) + GitHub App, webhooks via gosmee relay |
| `cicd-verdaccio` | Cache-only NPM registry for TaskRuns (no auth, in-cluster only) |
| `tenant-platform` | PaC tenancy: one registry entry → tenant namespace, quota, LimitRange, Cilium egress policy, `tekton-bot` SA, `npm-npmrc`, Repository CRs and developer RBAC |
| `cicd-argocd` | GitOps CD: ArgoCD v3.5.3 (chart 10.9.2), native OIDC SSO (Cognito/AWS SSO), app-of-apps from the `home-cluster` repo |

Operator runbook: [`developer/docs/cicd/pipelines/tenancy/runbook.md`](https://github.com/stijnklomp-talos-linux-home-setup/home-cluster/blob/main/developer/docs/cicd/pipelines/tenancy/runbook.md).


## Prerequisites

- Base layer deployed (`home-cluster-kubernetes-base`): the internal Istio edge Gateway + `home-issuer` (PaC URL), Longhorn (PVCs, TaskRun workspaces) and Cilium (tenant egress policy is only enforced on Cilium).
- CI node roles labeled (see `platform/node/designate-node-roles.yaml`).

## Deploy

In this order (PaC needs Tekton Pipelines from the operator):

```sh
cd cicd-tekton-operator && helmfile -l stage=operator apply && helmfile -l stage=config apply
cd cicd-tekton-pipelines-as-code && helmfile -l stage=expose-operator apply && helmfile -l stage=gosmee apply
cd cicd-verdaccio && helmfile apply
cd tenant-platform && helmfile -l stage=tenancy apply   # registry: values/tenants.yaml; owns Repository CRs; needs Cilium CRDs
cd cicd-argocd && helmfile -l stage=argocd apply   # needs secrets/argocd-secrets.yaml + argocd-github-creds.yaml (see its README)
```

For Pipelines-as-Code, also apply the GitHub App secret first (fill `secrets/pipelines-as-code-github-secrets.example.yaml` → `secrets/pipelines-as-code-github-secrets.yaml`).

## Cleanup

```sh
helmfile destroy
```

per directory, in reverse order.