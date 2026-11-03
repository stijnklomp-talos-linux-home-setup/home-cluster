# Terraform: AWS Cognito federation + IAM Identity Center (home)

Provisions the authentication backend for the home cluster in the home AWS
account (`220402652377`, eu-west-1). Two units share one S3 state bucket
(`home-terraform-state-220402652377`, S3-native locking):

- `prod/cognito` → `modules/cognito-federation`:
  - Cognito user pool `home-users` + domain `home-stijnklomp` + app clients (Pinniped, oauth2-proxy)
  - User groups `admin`, `service-a` mirroring cluster RBAC teams + owner user
- `prod/identity-center` → `modules/identity-center`:
  - IAM Identity Center permission sets `home-admin` / `home-readonly` + account assignments
  - Identity store group `home-admins` (owner `stijnklomp` member) — single principal for everything
  - Access-portal tiles (custom OIDC apps) for login / ArgoCD / Tekton — deep links to the
    Cognito-protected hosts (auth stays on Cognito; IIC is not an IdP here)

## Auth

`aws sso login --profile testing` first (SSO profile, never static keys). Never
commit credentials; Cognito app client secrets are `sensitive` outputs and must
be copied manually into:

- `home-cluster-internal/kubernetes/cicd/home-sso-auth-rbac/secrets/supervisor-oidc-secret.yaml`
- `home-cluster-internal/kubernetes/cicd/home-sso-auth-rbac/secrets/oauth2-proxy-secret.yaml`

## Apply

```sh
cd prod/cognito
terragrunt init && terragrunt plan && terragrunt apply

cd ../identity-center
terragrunt init && terragrunt plan && terragrunt apply
```

Order matters: cognito first (tile URLs reference hosts that exist once the
cluster side is deployed; IIC tiles only link to them).

## Notes

- The IIC instance itself is enabled manually in the AWS console (one-time);
  Terraform only manages resources inside it.
- Portal tiles are shortcuts: clicking one opens the Cognito login for that UI.
- Callback URLs match the `home-sso-auth-rbac` hosts (`login`/`argocd`/`tekton` at
  `192.168.1.200.sslip.io`).
- **Run terraform with the `home-admin` SSO role** (`sso_role_name = home-admin`
  in `~/.aws/config`): account-assignment provisioning runs under the caller
  identity and needs IAM write (`iam:GetSAMLProvider`, `iam:CreateRole`, ...).
  `PowerUserAccess` is NOT sufficient — first assignment must be bootstrapped
  from the console (root/admin), then imported:
  `terragrunt import aws_ssoadmin_account_assignment.admin <PRINCIPAL_ID,PRINCIPAL_TYPE,TARGET_ID,TARGET_TYPE,PERMISSION_SET_ARN,INSTANCE_ARN>`.