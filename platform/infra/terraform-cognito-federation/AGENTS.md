<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-03 -->

# AGENTS.md (platform/infra/terraform-cognito-federation)

AWS-side authentication for the home cluster in the home AWS account (`220402652377`, eu-west-1): Cognito user pool, app clients (Pinniped, oauth2-proxy), user groups mirroring cluster teams, AND IAM Identity Center (permission sets + access-portal tiles).

## Overview
Overview of this component — see `README.md` for prose.

## Setup
| Plans / apply | `terragrunt init` / `terragrunt plan` / `terragrunt apply` (in `prod/cognito` or `prod/identity-center`) |


## Commands (run inside the unit dir)

| Task | Command |
|------|---------|
| Init / Plan / Apply | `terragrunt init` / `terragrunt plan` / `terragrunt apply` |
| Cleanup | `terragrunt destroy -auto-approve` |

## Structure

- `root.hcl` — locals (`project`, `region` `eu-west-1`, `aws_account_id`), `remote_state` (S3 bucket `home-terraform-state-<account-id>`, per-unit keys, `use_lockfile`), `generate versions/providers` (Terraform >= 1.15.9, aws `~> 6.64.0`).
- `modules/cognito-federation/` — pool, domain, app clients, groups, owner user, **SAML IdP for AWS SSO** (`saml/` vendored metadata); `outputs.tf` exposes client ids/secrets (`sensitive`).
- `modules/identity-center/` — permission sets (`home-admin` AdministratorAccess, `home-readonly` ReadOnlyAccess), identity store group `home-admins`, account assignments, portal tiles (**argocd** + **tekton** = console-created IIC SAML apps, imported; OIDC custom-app tiles are broken in the portal — never go back to them). "Home Cluster Login" tile was removed (Pinniped is OIDC-only, no SAML tile possible).
- `prod/cognito/` + `prod/identity-center/` — one unit per module, both include `root.hcl`.

## Conventions & rules

- The IIC instance is enabled manually in the console (one-time) — the module only manages resources inside the existing instance; `data.aws_ssoadmin_instances` finds it.
- `home-admins` group is the SINGLE principal for account assignments and app assignments — add members by group membership, not per-user resources.
- **Run terraform with the `home-admin` SSO role** (profile `testing` uses `sso_role_name = home-admin`): account-assignment provisioning runs under the caller and needs IAM write (`iam:GetSAMLProvider`, `iam:CreateRole`, ...). PowerUserAccess is NOT sufficient; the first assignment must be bootstrapped via the console (root/admin) and `terragrunt import`ed (ID order: `PRINCIPAL_ID,PRINCIPAL_TYPE,TARGET_ID,TARGET_TYPE,PERMISSION_SET_ARN,INSTANCE_ARN`).
- **Cognito hosted UI (all REQUIRED, each one broke the login page until fixed):**
  - App clients need `supported_identity_providers` non-empty — API-created clients default to empty and the login route 400s "Invalid request". The **oauth2-proxy client is `["AWSSSO"]` ONLY** (web UIs always use AWS SSO; revert to `["COGNITO","AWSSSO"]` for a password fallback if AWS SSO is down); the **pinniped client stays `["COGNITO"]`** (kubectl password login).
  - The domain needs `managed_login_version = "2"` AND a `aws_cognito_managed_login_branding` per client (`use_cognito_provided_values = true`) — API-created clients get no branding style, so the hosted UI renders `/error`.
  - Do NOT add `offline_access` to `allowed_oauth_scopes` (ScopeDoesNotExistException) — the Pinniped OIDCIdP overrides `additionalScopes` to `[profile, email]` instead; Cognito returns refresh tokens for the code grant anyway.
- **SAML federation (IIC -> Cognito -> oauth2-proxy), all learned the hard way:**
  - `IdpIdentifiers` is NOT a valid `provider_details` key for SAML IdPs (OIDC-only) — app clients reference the SAML IdP by its **provider name** (`AWSSSO`); the client needs `depends_on` on the IdP.
  - SAML custom apps are **console-created** (`CreateApplication` API is OAuth-only) then `terragrunt import`ed. `UpdateApplication` AND `PutApplicationAssignmentConfiguration` are REJECTED for `custom-saml` apps ("not supported for this action") — the app object + assignment-required setting are console-managed; terraform holds `lifecycle.ignore_changes` and owns only the group assignment.
  - SAML apps default to IdP-initiated sign-in; an unsolicited assertion to Cognito 400s (`Invalid samlResponse or relayState`). The working tile behavior = set the app's **Application start URL** to the dashboard host (portal redirects the browser there; SP-initiated SSO then completes). Don't fight the tile origin.
  - The OIDC `custom` provider's portal tile is broken in the current portal (`ListProfilesForApplication` 404, "You don't have access to this application") — don't go back to it for tiles.
  - IdP metadata is vendored per IIC app in `modules/cognito-federation/saml/` (public cert — safe to commit; re-download on cert rotation).
- `aws_identitystore_group.home_admins.group_id` (UUID) is the attribute to use for membership/principal references — `.id` is a composite `store/group` string that the SSO APIs reject.
- Portal tiles: OIDC custom apps need `portal_options.sign_in_options.origin = "APPLICATION"`; SAML apps use the start-URL mechanism above.
- Callback URLs must match the ingress hosts used by `home-sso-auth-rbac` (login + argo/oauth2/callback at `192.168.1.200.sslip.io`).
- Groups here mirror `cluster-rbac` teams (admin, service-a) — keep in sync.
- Never commit AWS credentials; outputs are `sensitive` and flow by hand into `home-sso-auth-rbac/secrets/*.yaml`.
- Cross-repo rules: workspace root `AGENTS.md`.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- The account id (`220402652377`) and region are not secrets (they live in `~/.aws/config`); AWS access is via the `testing` SSO profile.

## Checklist
- [ ] helmfile chart paths / terragrunt sources resolve (no dangling `./charts/...`)
- [ ] YAML parses (`helm template` or a yaml lint)
- [ ] 3-way sync files unchanged unless intentionally updated together
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value

## Examples
`home-cluster-internal/terraform/test-terraform-apply-job` is the reference plan/apply CI flow.

## When stuck
- For pipeline/auth issues: `developer/docs/cicd/pipelines/troubleshoot/` and the restricted wiki runbooks.
