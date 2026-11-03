# NPM dependency cache: Verdaccio

## Status

Accepted (2026-09)

## Context

Tekton `npm-install-v1` tasks hit `registry.npmjs.org` directly on every run. Pipelines were slow and repeated the same downloads; no cache existed despite root AGENTS.md originally ruling out Verdaccio (auth kept cloudless — no publishing registry).

## Decision

Run **Verdaccio as a cache-only proxy** (`platform/cicd/cicd-verdaccio`):

- In-cluster ClusterIP service, Longhorn PVC (20 Gi), anonymous read, publishing disabled (`max_users: -1`).
- TaskRuns point their per-team `.npmrc` at `http://verdaccio.verdaccio.svc.cluster.local:4873`.
- Cache TTL: a CronJob evicts files older than 90 days twice a day (mtime-based — re-fetched packages reset their TTL); deleted entries are re-fetched from npmjs on demand.

## Rationale

- One registry for all teams/pipelines, no auth to operate, no publish surface — purely a performance/latency optimisation, not a distribution platform.
- 90-day TTL bounds the PVC while keeping actively-used packages warm.

## Consequences

Easier: faster repeat installs, LAN-only traffic after warm-up, storage bounded by the eviction job. Harder: first fetch of a new package still goes to npmjs; the cache is a single point of failure for installs (if Verdaccio is down, installs fail — acceptable for a home cluster; a restart is quick).