<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: opencode | Last updated: 2026-11-01 -->

# AGENTS.md (cicd-tekton-pipelines-as-code)

Git-triggered CI: Pipelines-as-Code (v0.50.0) + GitHub App (personal org). PaC runs the `.tekton/pipelinerun.yaml` from each repo in the namespace of the Repository CR that matched the event. Repository CRs are owned by `tenant-platform` and live in tenant namespaces; PaC auto-configure is disabled, so the registry is the only way a repository becomes PaC-enabled. A PipelineRun can still override its run namespace with the `pipelinesascode.tekton.dev/target-namespace` annotation (a Repository CR must exist there, and it must pass the tenancy admission gate).

## Overview
Overview of this component — see `README.md` for prose.

## Setup (deploy & teardown)

| Task | Command |
|------|---------|
| Expose | `helmfile -l stage=expose-operator apply` |
| Webhook relay (gosmee) | `helmfile -l stage=gosmee apply` — GitHub → `hook.pipelinesascode.com` → PaC controller (no domain needed) |
| Repository CRs | `cd ../tenant-platform && helmfile -l stage=tenancy apply` (this component no longer renders Repository CRs) |
| GitHub App secret | `kubectl apply -f secrets/pipelines-as-code-github-secrets.yaml` (fill from example first) |

## Commands
Full command table lives in the Setup section; offline validation: `helm template <rel> ./charts/<name>` and `helm lint` via the alpine/helm container (see workspace root AGENTS.md).

## Key files

| File | Notes |
|------|-------|
| `kustomizations/upstream/pipelines-as-code-0.50.0.yaml` | upstream release manifest (refresh via documented curl in README) |
| `kustomizations/pipelines-as-code/patches/configmaps.yaml` | controller URL (`pipelines-as-code.192.168.1.200.sslip.io`) + dashboard URL + **PaC settings**: `auto-configure-new-github-repo: "false"`, auto-configure templates deleted (`null`), `require-ok-to-test-sha` |
| `charts/pipelines-as-code-repositories/` | **deleted** (ticket #8) — Repository CRs come only from `tenant-platform`; the old in-cluster release was uninstalled. Do not recreate this chart. |
| `charts/expose-pipelines-as-code/` | HTTPRoute `pipelines-as-code` -> `pipelines-as-code-controller:8080`, attached by `sectionName: https-pac` to the internal Istio gateway; TLS terminates at the Gateway |
| `charts/gosmee-client/` | webhook relay client (ghcr.io/chmouel/gosmee:v0.32.0, pinned); forwards `hook.pipelinesascode.com` → PaC controller; reads `smee` from the secret |
| `secrets/pipelines-as-code-github-secrets.example.yaml` | GitHub App id/key/webhook + `smee` relay URL placeholders |

## Conventions & rules

- Edge cutover (ticket #12): PaC is served by the Istio Gateway `istio-internal` (`https-pac` listener) through `charts/expose-pipelines-as-code` (HTTPRoute, no Ingress/Certificate). Hostname and controller URL must stay `pipelines-as-code.192.168.1.200.sslip.io`; TLS is terminated at the Gateway.
- Repository CRs are owned by the `tenant-platform` registry (`../tenant-platform/values/tenants.yaml`), not by this component. Never add a Repository template here; the old `pipelines-as-code-repositories` chart and release were deleted in ticket #8.
- Auto-configure is disabled (`auto-configure-new-github-repo: "false"`). An unregistered owner that installs the App creates no namespace, Repository CR or PipelineRun. Do not re-enable it to onboard a repository — add a registry entry instead.
- The tenant-platform `ValidatingAdmissionPolicy` rejects a Repository whose `spec.url` owner does not match the namespace `home.cluster/tenant` label. Test changes with `../tenant-platform/tests/admission-checks.sh`.
- No `settings.policy` anywhere: PaC GitHub policies are TEAM slugs (org-only; on a personal account the team lookup 404s) and a set policy short-circuits the built-in ACL. Single-user repos rely on the built-in ACL (repo owner/collaborator/org member/push permissions); external whitelists go in a root `OWNERS` file (PaC reads `OWNERS` from the default branch, NOT `.tekton/OWNERS`).
- Per-run namespace override: `pipelinesascode.tekton.dev/target-namespace: "<ns>"` on the PipelineRun + a Repository CR for the repo in `<ns>` (add `<ns>` to the TektonConfig cluster-resolver allow-list too). In the tenancy model the run namespace is already the tenant namespace, so this is an exception path.
- Upstream manifest must be re-downloaded on bump, never edited in place.
- gosmee is the no-domain webhook path; when a domain + Cloudflare Tunnel exist, remove the `gosmee` release and point the GitHub App webhook at the domain instead.
- The GitHub App `repository` event is now optional: it only matters for auto-configure, which is off. Keep `push`, `pull_request`, `check_run`, `check_suite`, `commit_comment` and `issue_comment`.
- Cross-repo rules: workspace root `AGENTS.md`.

## Security
- No real secrets, keys, or kubeconfigs may be committed; `*.example.yaml` + `.gitignore` are the only allowed pattern.
- `<ingress-ip>`, `<your-github-org>`, `<region>`, `<user-pool-id>` are TODOs, not values to invent.

## Checklist
- [ ] helmfile chart paths / terragrunt sources resolve (no dangling `./charts/...`)
- [ ] YAML parses (`helm template` or a yaml lint)
- [ ] no Repository CR is rendered by this component (the registry owns them)
- [ ] auto-configure stays off and the template keys stay deleted
- [ ] No real secrets added (only `*.example.yaml`)
- [ ] No placeholder replaced with a fabricated value

## Examples
`cicd-tekton-operator` is the reference helmfile pattern; `tenant-platform` shows the registry-driven approach.

## When stuck
- For pipeline/auth issues: `developer/docs/cicd/pipelines/troubleshoot/` and the restricted wiki runbooks.
