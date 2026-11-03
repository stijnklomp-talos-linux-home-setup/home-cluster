# TODO — home-cluster

Open work for the cluster and this repo. Newest section first.

## Monorepo restructure (2026-11-03) — local work done

The 14 repos were consolidated into this repo plus the private
`home-cluster-internal`. The old repos are archived as git bundles at
`~/developer/personal/legacy-repos-20261103/`.

Done locally (uncommitted/pending push):

- Trees moved into `platform/` and `developer/`; nested `.git` directories removed.
- ArgoCD rewired: ApplicationSets and `bootstrap.yaml` point at `home-cluster`,
  declarations moved to `developer/apps/`, one repo credential entry.
- Declarations repointed at the monorepo (`chartPath: developer/examples/<app>/deploy/<app>`).
- `tenant-platform` registry collapsed to the two repos; offline checks pass.
- Pipeline defaults, scaffolding script and docs updated to the new paths.

Still to do:

- [ ] Push both repos (see the command list handed over with this change).
- [ ] Install the PaC GitHub App on `home-cluster` and `home-cluster-internal`,
      or the `Repository` CRs never receive events.
- [ ] Re-apply `cicd-argocd` (`helmfile -l stage=argocd apply`) so ArgoCD picks
      up the new repo URL and credential entry.
- [ ] Apply `tenant-platform` (`helmfile -l stage=tenancy apply`); it prunes the
      15 retired `Repository` CRs.
- [ ] Re-apply `bootstrap.yaml` (its `path` changed to `platform/gitops/applications`).
- [ ] Archive the 14 old GitHub repos once the new ones are pushed and synced.
- [ ] Close issues #10–#18 in the old `wiki` repo (they were never closed; the
      agent token had no write scope).

## GitHub credentials for ArgoCD + Tekton

**Problem:** fine-grained GitHub PATs (`github_pat_…`) are owner-scoped — one
owner per token. There is no multi-owner fine-grained token, so a single
fine-grained PAT cannot cover a personal account and an org at once.

**Decision (2026-09-22): Option A — one classic PAT (`ghp_…`, `repo` scope).**
Covers personal repos and every org the user belongs to. Trade-off: no
per-repo restriction; acceptable for a private home lab.

- [ ] Mint a classic PAT with `repo` scope (do not paste it in chat).
- [ ] Put it in `platform/cicd/cicd-argocd/secrets/argocd-github-creds.yaml`
      (`stringData.password`; keep `username`).
- [ ] Put it in
      `developer/pipelines/charts/gitops-yaml-bump-secrets/templates/gitops-pat-secret.yaml`
      (copy the example; keep `username: tekton-bot`). The `-secret.yaml` suffix
      is what gitignore matches.
- [ ] Re-apply the secrets and `cicd-argocd` helmfiles.

Rejected alternative (Option B, per-owner fine-grained PATs) is recorded in git
history: it needs one credential template per owner plus a `GITOPS_AUTH_SECRET`
parameter on `gitops-yaml-bump-v1`.

## Re-add a PaC Repository policy

`settings.policy` was removed from the PaC `Repository` CRs (2026-09-20) because
GitHub policy entries are org team slugs, and the personal repo
`stijnklomp-LAVE/test-app` 404s on team lookup — a set policy short-circuits
PaC's built-in ACL, so `/test` was denied.

**Current posture:** built-in ACL only (repo owner / collaborator / org member /
push access) plus an optional root `OWNERS` file (PaC reads `OWNERS` from the
repo root, not `.tekton/OWNERS`). Acceptable for a single user, broader than an
explicit whitelist.

To harden:

1. Create a GitHub team (`ci-approvers`) in the org with the trusted members.
2. Add `settings.policy` back in the tenant registry so it renders onto every
   `Repository` CR: `pull_request: [ci-approvers]`, `ok_to_test: [ci-approvers]`.
3. Personal-account repos cannot use teams — move them under the org or rely on
   the root `OWNERS` file.
4. Verify on a PR: `/test` as an authorised user, `/ok-to-test <sha>` for an
   external contributor. A wrong policy denies all matching events — there is no
   built-in-ACL fallthrough.

## Switch the GraphQL stack to production Cognito

- [ ] Create a **dedicated GraphQL API app client** in the Cognito user pool (or
      deliberately reuse one); the current three client ids belong to Pinniped,
      oauth2-proxy and ArgoCD.
- [ ] Fill `<region>`, `<pool>` and `<client-id>` in the chart values:
      `developer/examples/home-users/deploy/home-users/values.yaml`,
      `developer/examples/home-orders/deploy/home-orders/values.yaml`,
      `developer/examples/home-graphql/deploy/home-graphql/values.yaml`.
- [ ] Define scopes `users:read`, `orders:read`, `orders:admin`; add
      `custom:user_id`, `email` and `email_verified: true` to the **access** token
      via a pre-token-generation Lambda or custom-attribute mapping.
- [ ] Deploy the three charts **without** `values-verification.yaml`. The live
      cluster currently uses the test issuer/JWKS and
      `client_id: home-test-client`; that is verification-only.
- [ ] The gateway refuses to start while `JWT_ISSUER` is unset or still contains
      `<...>` — intentional; replace the placeholders first.
- [ ] Run the interactive login check in
      `developer/docs/operations/graphql-auth.md` with a real access token (not
      an ID token). Confirm: self-email without `users:read`, another user's
      email denied, another owner's order null.

## GitOps follow-ups

- [ ] The GraphQL stack is **single-environment**: charts, Service DNS and
      Cilium policies use fixed `home-users` / `home-orders` / `home-graphql`
      namespaces, while the generator produces `<app>-<env>` names. Documented in
      `platform/gitops/README.md`; template per environment before claiming a
      real dev/staging/prod rollout.
- [ ] The GraphQL declarations set `valuesFile: values-verification.yaml`. For
      production, fill the chart values and set `valuesFile: ""`.
- [ ] `supergraph-release-v1` writes the ArgoCD-managed
      `home-graphql-supergraph` ConfigMap **directly**. After a live release,
      commit the refreshed `developer/examples/home-graphql/schemas/*.graphql`,
      `supergraph.graphql` and
      `developer/examples/home-graphql/deploy/home-graphql/files/supergraph.graphql`,
      then Sync. Syncing before that commit reverts the live schema. Documented
      deviation: `developer/pipelines/charts/supergraph-release/README.md`.
- [ ] Postgres has **no** CiliumNetworkPolicy: a 5432-only policy was attempted
      and blocked DB traffic in this Cilium + retained-kube-proxy setup, so it
      was removed. Revisit once the selector/SNAT behaviour is understood; do
      not ship a DB policy until it is tested against the live Service.

## Future

- [ ] EKS: register `argocd cluster add <context> --name eks`, then add
      declarations under `developer/apps/eks/` (the ApplicationSet stub exists).
- [ ] Optional latency tuning: lower ArgoCD's `timeout.reconciliation` if the
      3-minute cache expiry delays generated Application updates.
- [ ] Move actual personal secrets out of the working tree: a `secrets/` file
      with a live Docker Hub token sits in
      `developer/pipelines/charts/docker-build-publish-secrets/templates/`
      (gitignored, never committed) — rotate it if it may have been exposed, and
      consider keeping live secrets outside the repo entirely.
