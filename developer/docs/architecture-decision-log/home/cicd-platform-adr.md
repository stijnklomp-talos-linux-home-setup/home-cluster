# CI/CD platform: Tekton

## Status

Accepted (2026)

## Context

The home cluster needs a CI/CD platform that is cloud-agnostic, self-hosted, and can drive any kind of build/test/deploy workload (container images, IaC, arbitrary scripts). GitHub Actions was considered but locks CI into a cloud provider and per-minute pricing.

## Decision

Use Tekton (Pipelines-as-Code for git-triggered runs) installed via the Tekton Operator (helm chart 0.81.1). Pipelines run inside `home-cluster-1` with cluster-resolver sharing across `tekton-pipelines` and `home-infra` namespaces (superseded 2026-09: single run namespace `tekton-pipelines` — see [pac-run-namespace-adr.md](./pac-run-namespace-adr.md); superseded 2026-10: one run namespace per GitHub owner — see [pac-tenancy-adr.md](./pac-tenancy-adr.md)). The Tekton Dashboard (v0.72.0) is installed standalone (not operator-managed) in **write-mode** behind the existing Cognito → oauth2-proxy auth chain.

## Rationale

A robust and proven platform; fully open-source; runs entirely on-cluster; RBAC via Kubernetes.

Dashboard exposure follows the ArgoCD pattern (oauth2-proxy reverse-proxy mode), extended with **per-user Kubernetes impersonation**: the proxy injects `Authorization: Bearer <its own SA token>` (long-lived, from the `tekton-oauth2-proxy-token` secret) plus `Impersonate-User` (the Cognito email claim). The dashboard forwards both to the API server, which evaluates the impersonated user's RBAC — so every user sees and writes exactly what their kubectl identity allows (`cluster-rbac`). The dashboard's own SA holds zero privileges (its upstream ClusterRoleBindings are stripped, per Tekton's least-privilege recipe), which is what makes write-mode safe: writes are per-user, not shared-SA. It is standalone rather than operator-managed because the operator would re-create those cluster-wide bindings.

## Consequences

Easier: full UI/logs on the cluster, no vendor lock-in. Harder: no managed runners; log retention (Results/Vector/MinIO) still TODO.

- Dashboard is **write-mode** (create/cancel/rerun/delete runs, import from git) but writes are scoped by each user's RBAC; PaC remains the git-driven path for CI runs — the dashboard is the manual/ad-hoc path (unversioned by design).
- The `tekton-oauth2-proxy` SA can impersonate any authenticated user (`oauth-proxy-impersonator` ClusterRole) — bounded by Cognito authentication, same trust model as ArgoCD.
- The proxy SA token is a long-lived secret (`tekton-oauth2-proxy-token`) — rotate manually; see `home-sso-auth-rbac/secrets/tekton-oauth2-proxy-token.example.yaml`.