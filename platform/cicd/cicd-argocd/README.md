# cicd-argocd — ArgoCD (GitOps CD)

ArgoCD v3.5.3 on `home-cluster-1`: deploys services declared in the
`home-cluster` GitHub repo. Each service has one declaration per target cluster;
an ApplicationSet generates `<app>-dev`/`-staging`/`-prod` (bare-metal now, EKS
later). Tekton CI builds images and bumps the env tags in `home-cluster`; dev and
staging auto-sync, prod is synced manually. SSO is native OIDC to Cognito (AWS
SSO only, per-user RBAC).

## Deploy

```sh
helmfile -l stage=argocd apply
```

Prerequisites:

- Cluster up with the Istio edge (`home-istio` Gateway `istio-internal`) and `home-issuer`.
- Terraform applied (`argocd` Cognito client exists — output `argocd_client_id`/`argocd_client_secret`)
- `secrets/argocd-secrets.yaml` (copy of the example, filled)
- `kubectl apply -f secrets/argocd-github-creds.yaml` (GitHub PAT, contents:read on the org repos)

## First login

- Browser: `https://argocd.192.168.1.200.sslip.io` → "Login via Cognito" → "Continue with AWS SSO"
- Break-glass local admin: password in `argocd-initial-admin-secret` (namespace `argo`)

## Layout

| Path | What |
|------|------|
| `charts/argo-cd-10.9.2.tgz` | vendored chart (app v3.5.3) |
| `charts/argocd-route/` | HTTPRoute `argocd` in `argo`, attached to the internal Istio gateway (`https-argocd`) |
| `values/argocd-values.yaml` | chart values (committed, no secrets); `server.insecure: true`, no Ingress/certificate |
| `secrets/` | OIDC creds + GitHub PAT (real files gitignored; `*.example.yaml` committed) |