# Authentication & access: AWS Cognito + Pinniped

## Status

Accepted (2026)

## Context

The cluster must be usable from the LAN without a public domain, while remaining authenticated. The previous setup used Pinniped + oauth2-proxy against an AWS Cognito pool. A new AWS account (and Cognito pool) will be provisioned with Terraform for this cluster.

## Decision

- **kubectl**: Pinniped Supervisor (Cognito as OIDCIdP) → Concierge impersonation proxy → per-user RBAC.
- **Web UIs** (ArgoCD, Tekton Dashboard): oauth2-proxy in front, forwarding `Impersonate-User`; login via **AWS SSO (IAM Identity Center) as a SAML IdP to the Cognito pool** — the oauth2-proxy client offers only the "Continue with AWS SSO" button (no password login for web UIs; the pinniped client keeps Cognito passwords for kubectl).
- **Portal tiles**: IIC SAML 2.0 applications (console-created, terraform-imported) whose **Application start URL** deep-links to each UI — clicking the tile opens the dashboard, then the one-click AWS SSO sign-in completes. (OIDC custom-app tiles are broken in the current portal — `ListProfilesForApplication` 404.)
- **TLS**: internal CA (`home-issuer`) via cert-manager for LAN; Cloudflare DNS-01 + tunnels reserved until a domain is purchased (code present, disabled).

## Rationale

Uses a proven auth model; Cognito keeps user/group management in AWS; Pinniped keeps native kubeconfig UX.

## Consequences

Easier: adding users = Cognito + RBAC values change. Harder: cert trust (internal CA must be distributed); importer/exporter endpoints for the kubeconfig require the Supervisor hostname to resolve on the LAN.