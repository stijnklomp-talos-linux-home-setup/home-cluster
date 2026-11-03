# CI/CD Tekton Pipelines as Code

Git-triggered CI via Pipelines-as-Code (v0.50.0) + a GitHub App in the personal org. GitHub webhooks reach the cluster via the gosmee relay (`hook.pipelinesascode.com`) — no domain or public IP needed.

## Prerequisites

- Cluster up, connected via Pinniped kubeconfig.
- Tekton Operator installed first (`cicd-tekton-operator`).
- LAN edge URL reachable through the Istio Gateway (`pipelines-as-code.192.168.1.200.sslip.io`).

## Download upstreams

Already vendored in `kustomizations/upstream/`. To refresh:

```sh
curl -L -o ./kustomizations/upstream/pipelines-as-code-0.50.0.yaml https://github.com/openshift-pipelines/pipelines-as-code/releases/download/v0.50.0/release.k8s.yaml
```

## Webhook relay (gosmee)

GitHub → `hook.pipelinesascode.com/<channel>` (public relay) → `gosmee-client` (in-cluster, outbound SSE) → PaC controller. The relay channel is random and unguessable; payloads are still HMAC-validated by the webhook secret.

1. Get a channel: open `https://hook.pipelinesascode.com/new` and copy the **Webhook Proxy URL**.
2. Put it in the secret as `smee` (see below).
3. The `gosmee-client` Deployment (stage `gosmee`) reads it from the secret.

## GitHub App

1. Create/verify the GitHub App in the personal org (`https://github.com/apps/pac-talos-linux-home-setup`):
   - Webhook URL = the relay URL from step 1 above (`https://hook.pipelinesascode.com/<channel>`)
   - Repository permissions: Checks `Read & Write`, Contents `Read & Write`, Issues `Read & Write`, Metadata `Read-only`, Pull requests `Read & Write`; Organization: Members `Read-only`
   - Events: `check_run`, `check_suite`, `commit_comment`, `issue_comment`, `pull_request`, `push`; `repository` is optional now that auto-configure is off.
2. Copy `secrets/pipelines-as-code-github-secrets.example.yaml` → `secrets/pipelines-as-code-github-secrets.yaml` and fill in app id, webhook secret, the app's private key, and `smee` (the relay URL).
3. Install the App on the org (and any other account/org that should run CI).

## Run namespace

Every PipelineRun runs in the namespace of the Repository CR that matched the event. Repository CRs are created by `tenant-platform` in the tenant namespace (e.g. `stijnklomp-lave`), so that namespace is the run namespace. It holds the tenant's `tekton-bot` service account, `npm-npmrc` ConfigMap and secrets; shared Tasks and Pipelines stay in `tekton-pipelines` and are resolved through the cluster resolver. A PipelineRun can still override its run namespace with the `pipelinesascode.tekton.dev/target-namespace: "<ns>"` annotation; PaC then looks up the Repository CR in `<ns>` (it must exist there, pass the admission gate) and `<ns>` must be in the TektonConfig cluster-resolver allow-list.

The old single-namespace path (`tekton-pipelines`) is retired: the `pipelines-as-code-repositories` chart and release are gone (chart deleted in ticket #8, release uninstalled), and the admission gate rejects Repository CRs in namespaces without a `home.cluster/tenant` label. `tekton-pipelines` holds shared Tasks/Pipelines and Tekton configuration only. Do not re-add the chart or release.

## Auto-configure

Auto-configure is **off** (`auto-configure-new-github-repo: "false"` in `kustomizations/pipelines-as-code/patches/configmaps.yaml`, and the `auto-configure-repo-*` template keys are deleted from the live ConfigMap). Installing the GitHub App on an unregistered owner creates no namespace, no Repository CR and no PipelineRun.

The tenant registry is the only way a repository becomes PaC-enabled:

```sh
cd ../tenant-platform
helmfile -l stage=tenancy apply
```

Authorization uses the built-in ACL (repo owner/collaborator/org member/push permissions) plus an optional root `OWNERS` file — no `settings.policy` is set (GitHub policy entries are team slugs, org-only, and a set policy short-circuits the built-in ACL). A Repository CR whose `spec.url` owner does not match the tenant namespace label is denied by the `tenant-platform-repository-owner` ValidatingAdmissionPolicy (defense-in-depth; the registry is the primary control).

## Deploy

```sh
kubectl apply -f secrets/pipelines-as-code-github-secrets.yaml

helmfile -l stage=expose-operator apply
helmfile -l stage=gosmee apply
```

Repository CRs are not rendered by this component; apply the tenant registry:

```sh
cd ../tenant-platform && helmfile -l stage=tenancy apply
```

## Cleanup

```sh
helmfile -l stage=gosmee destroy
helmfile -l stage=expose-operator destroy
helmfile -l stage=operator destroy

kubectl delete -f secrets/pipelines-as-code-github-secrets.yaml

kubectl delete namespace pipelines-as-code
```

Deleting the PaC namespace also deletes any global Repository CR (none exists in this cluster today); tenant Repository CRs live in tenant namespaces and are removed with `tenant-platform` (or the pruner).

## Moving to a domain (later)

When a domain + Cloudflare Tunnel exist: change the GitHub App's webhook URL to `https://pipelines-as-code.<domain>`, remove the `gosmee` release from `helmfile.yaml`, and add a public Gateway route with the public-issuer certificate (the internal `https-pac` route stays unchanged).
