# SAML IdP metadata (vendored, not secret)

One file per IIC SAML application — each IIC app has its own assertion
endpoint/cert, so each gets its own Cognito SAML IdP:

- `iic-tekton-metadata.xml` → IdP `AWSSSO` (used by the `oauth2-proxy` client, Tekton Dashboard)
- `iic-argocd-metadata.xml` → IdP `AWSSSO-ARGOCD` (used by the `argocd` client, ArgoCD)

## Adding an IIC SAML application (console)

IIC SAML apps are console-created (the CreateApplication API is OAuth-only).
For each new app:

1. IIC console → Applications → **Add application** → *IAM Identity Center SAML 2.0 application* (custom SAML 2.0)
2. Set **Application name** + **Application start URL** = the app's host
   (e.g. `https://argocd.192.168.1.200.sslip.io`) — the portal redirects the
   browser there and SP-initiated SSO completes (IdP-initiated assertions are
   rejected by Cognito: `Invalid samlResponse or relayState`).
3. Set **Application ACS URL** = `https://home-stijnklomp.auth.eu-west-1.amazoncognito.com/saml2/idpresponse`
   and **Application SAML audience** = `urn:amazon:cognito:sp:eu-west-1_hYEQJIIaf`
   (the Cognito SAML SP endpoints — same values as the Tekton app).
4. **Download IAM Identity Center SAML metadata file** (Actions menu) and save
   it as `<iic-<app>-metadata.xml>` in this directory — the
   `aws_cognito_identity_provider` resource reads it via `file()`.
5. Import the app into terraform (`prod/identity-center`):
   `terragrunt import aws_ssoadmin_application.<name> <application-arn>`
   (console apps reject `UpdateApplication`/`PutApplicationAssignmentConfiguration`,
   so terraform owns only the group assignment + `lifecycle.ignore_changes`).
6. In the cognito module: add the `aws_cognito_identity_provider` (provider
   name `AWSSSO_<APP>`) and point the app's Cognito client at it via
   `supported_identity_providers` + `depends_on`.
7. `terragrunt apply` in `prod/cognito` + `prod/identity-center`.

The files contain the IIC signing certificate (public) + SSO URL — safe to
commit. Re-download + replace when IIC rotates its signing certificate or the
app changes.