# AWS Cognito federation for the home cluster.
#
# Provisions:
# - Cognito user pool + domain + app clients (Pinniped, oauth2-proxy)
# - User groups mirroring the cluster teams (admin, service-a)
#
# IAM Identity Center (permission sets + portal tiles) lives in
# ../identity-center — Cognito here only.

resource "aws_cognito_user_pool" "home" {
  name = "home-users"

  # TODO: choose a real email; this is needed for password recovery etc.
  admin_create_user_config {
    allow_admin_create_user_only = true
  }

  schema {
    name                = "email"
    attribute_data_type = "String"
    mutable             = true
    required            = true
  }

  auto_verified_attributes = ["email"]

  username_configuration {
    case_sensitive = false
  }

  tags = {
    "managed-by" = "terragrunt"
    "purpose"    = "home-cluster-sso"
  }
}

resource "aws_cognito_user_pool_domain" "home" {
  domain       = "home-stijnklomp"
  user_pool_id = aws_cognito_user_pool.home.id
  # Managed login v2 (branding designer). v1 (classic hosted UI) is REQUIRED
  # to have a branding style per client, and API-created clients get none —
  # the hosted UI then renders /error ("An error was encountered with the
  # requested page.").
  managed_login_version = "2"
}

# --- App clients ---------------------------------------------------------

# Pinniped Supervisor (OIDC IdentityProvider)
resource "aws_cognito_user_pool_client" "pinniped" {
  name         = "pinniped"
  user_pool_id = aws_cognito_user_pool.home.id

  generate_secret = true

  # REQUIRED: the hosted UI login route 400s ("Invalid request") without the
  # built-in provider explicitly listed — console-created clients always set
  # this; API-created ones default to an empty list.
  supported_identity_providers = ["COGNITO"]

  # Localhost flow for `pinniped login oidc` CLI
  callback_urls = [
    "http://127.0.0.1:48095/callback",
    "https://login.192.168.1.200.sslip.io/callback",
  ]
  logout_urls = ["http://127.0.0.1:48095/"]
  allowed_oauth_flows_user_pool_client = true
  allowed_oauth_flows                  = ["code"]
  # NOTE: do NOT add "offline_access" here — Cognito rejects it with
  # ScopeDoesNotExistException. Refresh tokens come automatically from the
  # code grant (ALLOW_REFRESH_TOKEN_AUTH is set below); the Supervisor's
  # OIDCIdP must override additionalScopes to avoid requesting it.
  allowed_oauth_scopes                 = ["openid", "email", "profile"]
  explicit_auth_flows                  = ["ALLOW_USER_SRP_AUTH", "ALLOW_REFRESH_TOKEN_AUTH"]
}

# oauth2-proxy (web UI: Tekton Dashboard)
resource "aws_cognito_user_pool_client" "oauth2_proxy" {
  name         = "oauth2-proxy"
  user_pool_id = aws_cognito_user_pool.home.id

  generate_secret = true

  # Web UIs authenticate via AWS SSO ONLY (SAML IdP "AWSSSO"): the hosted UI
  # shows only the "Continue with AWS SSO" button — no password login. Password
  # login stays on the pinniped client (kubectl). If AWS SSO is unavailable,
  # web-UI logins are blocked — revert to ["COGNITO", "AWSSSO"] for a
  # password fallback.
  supported_identity_providers = ["AWSSSO"]
  # Client update needs the IdP to exist first (no implicit reference)
  depends_on = [aws_cognito_identity_provider.iic_saml]

  callback_urls = [
    "https://tekton.192.168.1.200.sslip.io/oauth2/callback",
  ]
  allowed_oauth_flows_user_pool_client = true
  allowed_oauth_flows                  = ["code"]
  allowed_oauth_scopes                 = ["openid", "email", "profile"]
  explicit_auth_flows                  = ["ALLOW_USER_SRP_AUTH", "ALLOW_REFRESH_TOKEN_AUTH"]
}

# ArgoCD (native OIDC, no oauth2-proxy in front): same AWS SSO chain — hosted
# UI shows only "Continue with AWS SSO". ArgoCD's callback path is /auth/callback.
resource "aws_cognito_user_pool_client" "argocd" {
  name         = "argocd"
  user_pool_id = aws_cognito_user_pool.home.id

  generate_secret = true

  # AWS SSO ONLY — but via its OWN SAML IdP (AWSSSO-ARGOCD): the ArgoCD portal
  # tile is a separate IIC SAML application with its own metadata (see
  # saml/iic-argocd-metadata.xml), unlike the oauth2-proxy client which uses
  # the Tekton app's IdP (AWSSSO).
  supported_identity_providers = ["AWSSSO-ARGOCD"]
  depends_on                   = [aws_cognito_identity_provider.iic_saml_argocd]

  callback_urls = [
    "https://argocd.192.168.1.200.sslip.io/auth/callback",
  ]
  allowed_oauth_flows_user_pool_client = true
  allowed_oauth_flows                  = ["code"]
  # NOTE: no offline_access (ScopeDoesNotExistException); ArgoCD gets refresh
  # tokens from the code grant anyway. No preferred_username claim in Cognito —
  # ArgoCD falls back to the email claim for the username (== cognito:username
  # in this pool).
  allowed_oauth_scopes = ["openid", "email", "profile"]
  explicit_auth_flows  = ["ALLOW_USER_SRP_AUTH", "ALLOW_REFRESH_TOKEN_AUTH"]
}

# --- SAML federation (AWS SSO tiles) --------------------------------------
# IAM Identity Center (AWS SSO) as a SAML IdP: portal tile -> IIC SAML ->
# Cognito (SAML SP) -> oauth2-proxy (OIDC, unchanged). The IIC SAML app is
# console-created (CreateApplication API is OAuth-only) — the IdP metadata
# below comes from its console download (see saml/README.md).

resource "aws_cognito_identity_provider" "iic_saml" {
  user_pool_id  = aws_cognito_user_pool.home.id
  provider_name = "AWSSSO"
  provider_type = "SAML"

  provider_details = {
    MetadataFile = file("${path.module}/saml/iic-tekton-metadata.xml")
    # NOTE: IdpIdentifiers is OIDC-only — NOT a valid key for SAML providers;
    # app clients reference this IdP by its provider_name ("AWSSSO").
  }

  # SAML assertion attributes -> pool attributes. email_verified literal
  # "true" (SAML has no verified flag; IIC users come from the identity store).
  attribute_mapping = {
    email          = "email"
    email_verified = "true"
  }
}

# Second IIC SAML IdP for the ArgoCD portal tile (separate IIC SAML
# application = separate metadata = separate IdP; the ArgoCD Cognito client
# references it as "AWSSSO-ARGOCD"). Metadata downloaded from the IIC console
# (see saml/README.md); same attribute mapping as the Tekton IdP.
resource "aws_cognito_identity_provider" "iic_saml_argocd" {
  user_pool_id  = aws_cognito_user_pool.home.id
  provider_name = "AWSSSO-ARGOCD"
  provider_type = "SAML"

  provider_details = {
    MetadataFile = file("${path.module}/saml/iic-argocd-metadata.xml")
  }

  attribute_mapping = {
    email          = "email"
    email_verified = "true"
  }
}

# --- Managed login branding (hosted UI will not render without it) --------
# App clients created via API get NO branding style; the hosted UI shows
# /error until one is assigned. use_cognito_provided_values = Cognito default.

resource "aws_cognito_managed_login_branding" "pinniped" {
  user_pool_id               = aws_cognito_user_pool.home.id
  client_id                  = aws_cognito_user_pool_client.pinniped.id
  use_cognito_provided_values = true
}

resource "aws_cognito_managed_login_branding" "oauth2_proxy" {
  user_pool_id               = aws_cognito_user_pool.home.id
  client_id                  = aws_cognito_user_pool_client.oauth2_proxy.id
  use_cognito_provided_values = true
}

resource "aws_cognito_managed_login_branding" "argocd" {
  user_pool_id               = aws_cognito_user_pool.home.id
  client_id                  = aws_cognito_user_pool_client.argocd.id
  use_cognito_provided_values = true
}

# --- User groups (mirror cluster teams) ----------------------------------

resource "aws_cognito_user_group" "admin" {
  name         = "admin"
  user_pool_id = aws_cognito_user_pool.home.id
  description  = "Cluster-wide admin access"
}

resource "aws_cognito_user_group" "service_a" {
  name         = "service-a"
  user_pool_id = aws_cognito_user_pool.home.id
  description  = "Team namespace for the (future) service-a"
}

# --- Users (placeholders) -------------------------------------------------

resource "aws_cognito_user" "owner" {
  user_pool_id = aws_cognito_user_pool.home.id
  username     = "stijnklomp1@hotmail.com"
  # TODO: use a real password / invite flow
  attributes = {
    email          = "stijnklomp1@hotmail.com"
    email_verified = "true"
  }
}

resource "aws_cognito_user_in_group" "owner_admin" {
  user_pool_id = aws_cognito_user_pool.home.id
  group_name   = aws_cognito_user_group.admin.name
  username     = aws_cognito_user.owner.username
}