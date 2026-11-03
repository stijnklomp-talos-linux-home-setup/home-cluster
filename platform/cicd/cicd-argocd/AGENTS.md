<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-01 -->

# AGENTS.md (cicd-argocd)

ArgoCD v3.5.3 (chart 10.9.2) — the GitOps CD layer of home-cluster-1. ApplicationSet machinery lives here; the declarations live in **developer/apps/** (one declaration per (app, cluster) → `<app>-dev`/`-staging`/`-prod`); this helmfile project is only the day-1 install path (ADR: helmfile for bootstrap, ArgoCD for everything after).

## Overview
Overview of this component — see `README.md` for prose.

## Setup (deploy & teardown)

| Task | Command |
|------|---------|
| Deploy | `helmfile -l stage=argocd apply` (needs `secrets/argocd-secrets.yaml` + `secrets/argocd-github-creds.yaml` first — see example) |
| Cleanup | `helmfile -l stage=argocd destroy` |

## Commands

| Task | Command |
|------|---------|
| Initial admin password | `kubectl get secret -n argo argocd-initial-admin-secret -o jsonpath='{.data.password}' \| base64 -d` (local admin is break-glass only; SSO is the login path) |
| Argocd CLI (login via SSO) | `argocd login argocd.192.168.1.200.sslip.io --sso` (server.insecure → port-forward instead if needed) |

## Key files

| File | Notes |
|------|-------|
| `charts/upstream/argo-cd-10.9.2.tgz` | vendored from argoproj/argo-helm (app v3.5.3) — refresh via `helm pull argo/argo-cd --version <x>`; update README + helmfile together |
| `charts/argocd-route/` | HTTPRoute `argo/argocd` -> `argocd-server:80`, attached by `sectionName: https-argocd` to `istio-internal`; TLS terminates at the Gateway |
| `values/argocd-route-values.yaml` | committed edge hostname (same value as `configs.cm.url`); no new address |
| `values/argocd-values.yaml` | worker-pool nodeSelector, **no Ingress and no server certificate** (the Gateway owns TLS), RBAC (`policy.default` readonly, admin by email), declarative repo creds (credential templates `github-creds` for the org + `github-creds-personal` for the personal account, both using the classic PAT from Option A), dex OFF, server.insecure |
| `secrets/argocd-secrets.example.yaml` | Cognito OIDC block (`oidc.config` + `oidc.argocd.oidcSecret`) + `argocd-github-creds` Secret manifest (GitHub PAT). Real file gitignored |

## Conventions & rules

- Edge cutover (ticket #12): ArgoCD is served by the Istio Gateway `istio-internal` (`https-argocd` listener) through `charts/argocd-route`. `server.insecure: true` stays; do **not** re-enable `server.ingress` or `server.certificate` — that would recreate an ingress-nginx Ingress and a competing TLS secret.
- The route lives in `argo` with its backend, so it needs no ReferenceGrant. The Gateway admits routes only from namespaces labelled `istio.io/edge-route: enabled` (set by `home-istio`).
- SSO is native OIDC → Cognito (`argocd` client, AWSSSO-only) — NOT oauth2-proxy (ArgoCD v3 has no header auth; the old expose-argo chart was removed). The Cognito callback stays `https://argocd.<lb-ip>.sslip.io/auth/callback`; do not change the hostname.
- Cognito quirks apply (see `home-sso-auth-rbac/AGENTS.md`): no `offline_access`, no `preferred_username` — ArgoCD usernames come from the `email` claim (== `cognito:username` in this pool); `rbac.csv` maps emails.
- `configs.cm.oidc.config` is a **block scalar from the secrets values file** — moving it back into committed values breaks the merge; keep the split (committed values has no placeholders, secrets file has the real clientID/secret).
- Credential templates match by URL prefix — `github-creds` (org) and `github-creds-personal` (personal account) share the same classic PAT secret, so new app repos need NO per-repo creds.
- Environment model (dev auto on merge, staging auto on `v*` tag, prod manual Sync) is encoded in the `home-cluster` ApplicationSets — see `developer/docs/architecture-decision-log/home/gitops-environments-adr.md`.
- Never commit `secrets/argocd-secrets.yaml` / `argocd-github-creds.yaml` (gitignore); PAT should be fine-grained (`contents: read` on the org repos).
- Version bump: chart 10.x ↔ app 3.x pairing — verify `appVersion` in the chart before bumping.
- GitOps source of truth: app manifests live in `home-cluster` — changes there are the deploy path, not this chart.
- Cross-repo rules: workspace root `AGENTS.md`.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- The admin password is random (chart) and local admin should stay disabled-or-break-glass; SSO users are the real identity layer.
- `<ingress-ip>`, `<your-github-org>`, `<region>`, `<user-pool-id>` are TODOs, not values to invent.

## Checklist
- [ ] helmfile chart paths / terragrunt sources resolve (no dangling `./charts/...`)
- [ ] YAML parses (`helm template` or a yaml lint)
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value

## Examples
`home-cluster-kubernetes-cicd/cicd-tekton-operator` is the reference helmfile pattern.

## When stuck
- For pipeline/auth issues: `developer/docs/cicd/pipelines/troubleshoot/` and the restricted wiki runbooks.
- ArgoCD operator manual: https://argo-cd.readthedocs.io/en/stable/operator-manual/