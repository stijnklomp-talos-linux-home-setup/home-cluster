# PaC run namespace: single `tekton-pipelines`

## Status

Superseded (2026-10) by [pac-tenancy-adr.md](./pac-tenancy-adr.md).

## Context

PaC ran each repo's PipelineRun in the namespace of its Repository CR: `home-auto` for auto-configured repos and the per-team namespaces `home-infra` / `home-admin` / `home-service-a` for listed repos. That meant five namespaces to operate — RBAC, pipeline secrets, `tekton-bot` ServiceAccounts and `npm-npmrc` ConfigMaps per namespace, plus a cluster-resolver allow-list entry for each. For a single-user home lab the team split added operational cost without an isolation benefit.

## Decision

- Every Repository CR lives in `tekton-pipelines`; PaC creates the PipelineRun there (the Repository CR namespace IS the run namespace). The `home-*` team namespaces are removed.
- One per-run escape hatch: `pipelinesascode.tekton.dev/target-namespace: "<ns>"` on a PipelineRun redirects PaC to a Repository CR in `<ns>` (the CR must exist there and `<ns>` must be in the TektonConfig cluster-resolver allow-list).
- Shared tasks, the `tekton-bot` SA, pipeline secrets and the `npm-npmrc` ConfigMap (Verdaccio cache) live in `tekton-pipelines`.

## Rationale

- PaC has no "default run namespace" knob — the Repository CR namespace is the run namespace, so consolidating CRs into one namespace is the supported way to consolidate runs.
- Bare minimum namespaces: one CI namespace instead of five; RBAC/secrets/tasks are managed once.
- The `target-namespace` annotation keeps a supported, per-run path back to multiple namespaces if team isolation is needed later.
- Alternatives considered: keeping the per-team namespaces (cost without benefit at this scale); a new `home-cicd` namespace (no reason to diverge from the Tekton control-plane namespace, which already holds the tasks and `tekton-bot` SA).

## Consequences

Easier: one namespace to inspect (dashboard pinned to it), one set of secrets, no namespace-per-repo plumbing; terraform plan/apply and app CI share the same run namespace. Harder: no namespace-level isolation between pipelines — anything allowed to trigger CI runs alongside everything else in `tekton-pipelines`; re-introducing team isolation needs a `target-namespace` override plus RBAC/secrets in the target namespace.
