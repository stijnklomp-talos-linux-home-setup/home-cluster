# GitOps: ArgoCD

## Status

Accepted (2026) — implemented 2026-09 with ArgoCD **v3.5.3** (chart 10.9.2).

## Context

We should have everything deployed with helmfile and vendored charts. At home, a single source of truth for the whole stack (base layer, CICD, auth) in git reduces drift and makes setup reproducible.

## Decision

Use helmfile for day-0 bootstrap (base layer), then ArgoCD (v3.5.3) for ongoing management of everything after via an App-of-Apps pattern. Each component repo exposes a Helm chart that ArgoCD renders. App manifests live in the `home-cluster` repo — one service declaration per (app, cluster) that an ApplicationSet expands into dev/staging/prod (see [gitops-environments-adr.md](./gitops-environments-adr.md)), so a future AWS EKS cluster is a declaration move away.

- **SSO**: native OIDC → Cognito (dedicated `argocd` app client, AWS SSO only) + `rbac.csv` per-user roles. NOT oauth2-proxy: ArgoCD v3 has no HTTP-header auth (Dex, which had the authproxy connector, was removed in 2.6).
- **CI → CD**: Tekton (PaC) builds → Docker Hub → the shared `app-release-v1` pipeline runs `gitops-yaml-bump-v1` to set the env tag/revision keys in `home-cluster` → ArgoCD ApplicationSet syncs the environment (dev/staging auto; prod manual).

## Rationale

ArgoCD gives auto-reconcile, diff visibility, and rollback; helmfile remains the documented day-1 install path to avoid chicken-and-egg problems. App-of-apps with per-cluster files keeps the multi-cluster switch (bare-metal ↔ EKS) declarative.

## Consequences

Easier: rolling back a broken update, replaying config on a fresh cluster, switching an app to EKS later. Harder: two deploy tools to know; ArgoCD is a new service (SSO + RBAC configured; no oauth2-proxy in front — see auth ADR).