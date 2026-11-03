# GitOps environments: dev / staging / prod from one declaration

## Status

Accepted (2026-09) — implemented in the rebuilt `home-cluster` GitOps repo
(ApplicationSets + per-service declarations).

## Context

Services need three environments on a chosen target cluster (bare-metal now,
EKS later) with distinct promotion triggers:

- merge to `main` → development
- tag on `main` → staging
- manual trigger → production

Maintaining per-environment Application manifests does not scale: each new
environment or cluster multiplies files, and the deploy semantics (auto-sync?
which image tag?) drift per app. We want to declare a service once and have the
three environments materialize consistently.

## Decision

- **One service declaration per (service, target cluster)** in the `home-cluster`
  repo: `developer/apps/<cluster>/<app>.yaml` with flat keys (`app`, `repoURL`,
  `chartPath`, `imageRepository`, `targetRevision<Env>`, `imageTag<Env>`).
  Adding/removing the file adds/removes all three Applications.
- **One ApplicationSet per cluster**: `applications/<cluster>.yaml` expands
  declarations × `[dev, staging, prod]` (matrix of git file generator and list
  generator), creating `<app>-dev` / `-staging` / `-prod` in namespace `argo`
  and target namespaces `<app>-<env>` (`CreateNamespace=true`).
- **Sync policy by environment**: dev/staging have `automated` (prune +
  selfHeal); prod has no `automated` policy, so a release lands as `OutOfSync`
  and a human clicks **Sync** in ArgoCD — that is the manual production gate.
  The conditional policy is applied via ApplicationSet `templatePatch`.
- **Triggers via Tekton PaC + the shared `app-release-v1` pipeline**:
  - PR → build only.
  - merge to `main` → build `:<sha>`, set `imageTagDev`.
  - `v*` tag → build `:<git_tag>`, set `imageTagStaging`/`targetRevisionStaging`
    and `imageTagProd`/`targetRevisionProd` to the tag.
  - The `gitops-yaml-bump-v1` task commits the changes with `[skip ci]`.
- **Reproducible envs**: dev tracks the app repo's `main`; staging and prod pin
  `targetRevision` to the release tag, so an environment deploys exactly the
  commit that was validated.
- **EKS switch**: the same ApplicationSet pattern with `destination.name: eks`
  (`applications/eks.yaml`); moving a declaration from `developer/apps/baremetal/` to
  `developer/apps/eks/` is the whole migration.
- **Credentials**: one classic PAT (`repo` scope) covers ArgoCD repo access and
  the tag-bump push (TODO.md Option A).

## Rationale

Declarations + ApplicationSet keep environment semantics in one place and make
"add a service" a single file. Alternatives considered: per-(app, env)
Application manifests (duplication, drift) and ArgoCD Image Updater
(registry-driven, no clear tag→staging mapping). Auto-recording the release for
prod while requiring a manual sync gives GitOps diff visibility with an
explicit human gate, and ArgoCD history provides rollback. Pinned revisions
make staging/prod immutable pointers to a validated release.

## Consequences

Easier: onboarding services, consistent env semantics, EKS migration, rollback,
audit trail (tag + GitOps commits). Harder: ApplicationSet polling adds latency
(configurable, ~1 min here) between the CI bump and the sync; flat declaration
key names are load-bearing for the bump task; prod intentionally shows
`OutOfSync` between a release and the manual sync.
