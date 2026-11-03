# AWS IAM Identity Center (AWS SSO) for the home AWS account.
#
# - Permission sets: home-admin (AdministratorAccess) + home-readonly (ReadOnlyAccess)
# - Identity store group `home-admins` (owner member) — the group is the
#   principal for ALL assignments below
# - Account assignments for both permission sets (this account)
# - Portal tiles: ArgoCD + Tekton Dashboard (both console-created IIC SAML
#   2.0 apps, imported). "Home Cluster Login" was removed — the Pinniped
#   Supervisor is OIDC-only, so no working tile is possible for it.
#
# The IIC instance itself is created in the AWS console (one-time, manual);
# this module only manages resources INSIDE the existing instance.

data "aws_ssoadmin_instances" "this" {}

data "aws_caller_identity" "current" {}

locals {
  instance_arn      = data.aws_ssoadmin_instances.this.arns[0]
  identity_store_id = data.aws_ssoadmin_instances.this.identity_store_ids[0]
  ingress_ip        = "192.168.1.200"
}

# --- Permission sets -----------------------------------------------------

resource "aws_ssoadmin_permission_set" "admin" {
  name             = "home-admin"
  description      = "Full administrator access to the home AWS account"
  instance_arn     = local.instance_arn
  session_duration = "PT8H"
}

resource "aws_ssoadmin_managed_policy_attachment" "admin" {
  instance_arn       = local.instance_arn
  permission_set_arn = aws_ssoadmin_permission_set.admin.arn
  managed_policy_arn = "arn:aws:iam::aws:policy/AdministratorAccess"
}

resource "aws_ssoadmin_permission_set" "readonly" {
  name             = "home-readonly"
  description      = "Read-only access to the home AWS account"
  instance_arn     = local.instance_arn
  session_duration = "PT8H"
}

resource "aws_ssoadmin_managed_policy_attachment" "readonly" {
  instance_arn       = local.instance_arn
  permission_set_arn = aws_ssoadmin_permission_set.readonly.arn
  managed_policy_arn = "arn:aws:iam::aws:policy/ReadOnlyAccess"
}

# --- Identity store: group + owner --------------------------------------

resource "aws_identitystore_group" "home_admins" {
  identity_store_id = local.identity_store_id
  display_name      = "home-admins"
  description       = "Administrators of the home cluster and AWS account"
}

data "aws_identitystore_user" "owner" {
  identity_store_id = local.identity_store_id

  alternate_identifier {
    unique_attribute {
      attribute_path  = "UserName"
      attribute_value = "stijnklomp"
    }
  }
}

resource "aws_identitystore_group_membership" "owner" {
  identity_store_id = local.identity_store_id
  group_id          = aws_identitystore_group.home_admins.group_id
  member_id         = data.aws_identitystore_user.owner.user_id
}

# --- Account assignments (principal = home-admins group) -----------------

resource "aws_ssoadmin_account_assignment" "admin" {
  instance_arn       = local.instance_arn
  permission_set_arn = aws_ssoadmin_permission_set.admin.arn
  principal_id       = aws_identitystore_group.home_admins.group_id
  principal_type     = "GROUP"
  target_id          = data.aws_caller_identity.current.account_id
  target_type        = "AWS_ACCOUNT"
}

resource "aws_ssoadmin_account_assignment" "readonly" {
  instance_arn       = local.instance_arn
  permission_set_arn = aws_ssoadmin_permission_set.readonly.arn
  principal_id       = aws_identitystore_group.home_admins.group_id
  principal_type     = "GROUP"
  target_id          = data.aws_caller_identity.current.account_id
  target_type        = "AWS_ACCOUNT"
}

# --- Custom SAML applications (access-portal tiles) ------------------------
# BOTH tiles are console-created IIC SAML 2.0 applications (the CreateApplication
# API is OAuth-only; OIDC custom-app tiles are broken in the current portal —
# ListProfilesForApplication 404, "You don't have access to this application").
# Terraform owns only the group assignment; the app object + assignment-required
# setting are console-managed. The tiles work via the app's **Application start
# URL** deep-linking to each UI host; SP-initiated SSO then completes.
# "Home Cluster Login" was removed (2026-09-12): the Pinniped Supervisor is
# OIDC-only, so no working tile is possible for it.

# ArgoCD — SAML 2.0 application (console-created 2026-09, imported). Start URL
# https://argocd.192.168.1.200.sslip.io; auth chain: IIC SAML -> Cognito
# (pool SAML IdP AWSSSO-ARGOCD, see cognito-federation module) -> ArgoCD
# native OIDC.
resource "aws_ssoadmin_application" "argocd_saml" {
  name                     = "ArgoCD"
  description              = "SAML 2.0 application federating IAM Identity Center to the Cognito user pool"
  application_provider_arn = "arn:aws:sso::aws:applicationProvider/custom-saml"
  instance_arn             = local.instance_arn

  portal_options {
    visibility = "ENABLED"

    sign_in_options {
      application_url = "https://argocd.${local.ingress_ip}.sslip.io"
      origin          = "APPLICATION"
    }
  }

  lifecycle {
    ignore_changes = [name, description, application_provider_arn, portal_options, status, tags]
  }
}

resource "aws_ssoadmin_application_assignment" "argocd_saml" {
  application_arn = aws_ssoadmin_application.argocd_saml.arn
  principal_id    = aws_identitystore_group.home_admins.group_id
  principal_type  = "GROUP"
}

# Tekton Dashboard — SAML 2.0 application (console-created: the
# CreateApplication API is OAuth-only; import it after console creation —
# `terragrunt import aws_ssoadmin_application.tekton <arn>`). Provider
# `custom-saml` is immutable AND console-created SAML apps
# reject UpdateApplication + PutApplicationAssignmentConfiguration ("not
# supported for this action") — the app object and its assignment-required
# setting are console-managed (default: assignment required). Terraform owns
# the group assignment only; the tile works via the app's **Application start
# URL** (console property) pointing at the dashboard host. Auth chain:
# IIC SAML -> Cognito (pool SAML IdP) -> oauth2-proxy.
resource "aws_ssoadmin_application" "tekton" {
  name                     = "Tekton Dashboard"
  description              = "SAML 2.0 application federating IAM Identity Center to the Cognito user pool"
  application_provider_arn = "arn:aws:sso::aws:applicationProvider/custom-saml"
  instance_arn             = local.instance_arn

  portal_options {
    visibility = "ENABLED"

    sign_in_options {
      application_url = "https://tekton.${local.ingress_ip}.sslip.io"
      origin          = "APPLICATION"
    }
  }

  lifecycle {
    ignore_changes = [name, description, application_provider_arn, portal_options, status, tags]
  }
}

resource "aws_ssoadmin_application_assignment" "tekton" {
  application_arn = aws_ssoadmin_application.tekton.arn
  principal_id    = aws_identitystore_group.home_admins.group_id
  principal_type  = "GROUP"
}