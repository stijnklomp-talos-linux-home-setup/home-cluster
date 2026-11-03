# Access

Everything is authenticated through AWS Cognito (OIDC) with a new app client for this cluster.

## Web UIs

| Service | URL (LAN) | Notes |
|---|---|---|
| ArgoCD | `https://argocd.<lb-ip>.sslip.io` | native OIDC → Cognito (AWS SSO only); per-user RBAC (`rbac.csv`); **Login via Cognito → Continue with AWS SSO** |
| Tekton Dashboard | `https://tekton.<lb-ip>.sslip.io` | via oauth2-proxy + Cognito; per-user RBAC (impersonation), write-mode; sign in with **Continue with AWS SSO** (IIC SAML) |

Replace `<lb-ip>` with the MetalLB address of the internal Istio edge Gateway
(`Gateway` `istio-internal` in namespace `istio-ingress`). The hostnames did
**not** change in the ingress-nginx → Istio cutover (ticket #12), so existing
bookmarks and Cognito callbacks keep working. TLS for the three HTTP UIs
terminates at the gateway with the multi-SAN `internal-tls` certificate;
Pinniped is TLS passthrough and serves its own certificates.

When a domain + Cloudflare Tunnel exist, a `https://<svc>.<domain>` list will
replace this table. Public exposure is deferred; the public route/gateway work
is tracked in the [cutover runbook](../../operations/istio-ambient-cutover.md).

First time here (or after a CA rotation)? See [Trusting the cluster certificate](ca-trust.md) so your device stops showing "untrusted" warnings.

## kubectl (Pinniped)

The Pinniped-flavored kubeconfig lives in `home-cluster-internal/kubernetes/sso-auth-rbac/kubeconfigs/` (restricted repo).

The supervisor (`login.<lb-ip>.sslip.io`) and the concierge impersonation
proxy (`kubectl.<lb-ip>.sslip.io`) are published through Gateway API
`TLSRoute` objects on the same gateway, with TLS **passthrough**. The
hostnames are unchanged from the ingress-nginx era.

Use the `pinniped` CLI (v0.47.0+) for initial login:

```sh
pinniped login oidc \
  --upstream-identity-provider-name=cognito \
  --request-audience=kubectl.<login-host> \
  --scopes=offline_access,openid,pinniped:request-audience,username,groups
```

Your RBAC is derived from your Cognito email (see `home-sso-auth-rbac/charts/cluster-rbac/values.yaml`).
