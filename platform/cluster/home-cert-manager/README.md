# cert-manager + issuers

Issues and renews TLS certificates. `home-issuer` (internal CA) covers all LAN hosts via `.sslip.io` addresses. Cloudflare DNS-01 issuer is reserved for a future public domain (see commented file in `kustomizations/home-issuers/`).

## Deploy

```sh
helmfile apply
```

## Trusting the internal CA

The root CA is stored in the Secret `home-root-ca` (namespace `cert-manager`):

```sh
kubectl -n cert-manager get secret home-root-ca -o jsonpath='{.data.tls\.crt}' | base64 -d > home-ca.crt
```

Install `home-ca.crt` in the OS trust store of every device that browses the cluster (or configure the browser to trust it). **Plain-language, per-OS instructions (incl. removing an old CA after a rotation): `developer/docs/cicd/access/ca-trust.md`.**

> Rotating the root CA (deleting the `home-root-ca` Secret) invalidates every issued cert and requires re-importing the CA on all devices — never rotate casually.

## Cleanup

```sh
helmfile destroy
```