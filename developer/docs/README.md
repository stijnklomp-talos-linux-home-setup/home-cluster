# Documentation

Design decisions, access guides and runbooks for `home-cluster-1`. Docs only —
no code. If you are looking for *how to deploy an app*, start at
[`../README.md`](../README.md) instead.

## Contents

- [Architecture decision log](architecture-decision-log/README.md) — ADRs (`home/` for this cluster, template in `templates/`)
- [Glossary](GLOSSARY.md) — tenancy, CI/CD and service-networking vocabulary
- [CI/CD access](cicd/access/README.md) — SSO login + kubeconfig setup via Pinniped/Cognito
- [Trusting the cluster certificate](cicd/access/ca-trust.md) — install/remove the `home-root-ca` on your devices (plain-language)
- [Pipelines](cicd/pipelines/setup/README.md) — how to run Tekton pipelines in git repos
- [Pipeline tips](cicd/pipelines/tips/README.md) — practical patterns
- [Pipeline troubleshooting](cicd/pipelines/troubleshoot/README.md) — when a run does not start or fails
- [PaC tenancy runbook](cicd/pipelines/tenancy/runbook.md) — add/remove a tenant, secrets, developer visibility, network policy, rollback
- [PaC tenancy spec](cicd/pipelines/tenancy/spec.md) — the multi-tenant contract
- [Istio edge + ambient cutover](operations/istio-ambient-cutover.md) — replacement for the retired ingress-nginx, with rollback
- [GraphQL JWT auth](operations/graphql-auth.md) — Cognito token flow, `@requiresScopes`, owner rules, direct subgraph access
- [Pipeline retention](cicd/pipelines/retention/README.md) — how long runs, logs and PVCs stick around
- [Tools](tools/jwt/README.md) — local JWT inspector via Docker

## Restricted material

Security-sensitive runbooks and ADRs are **not** here. They live in
`home-cluster-internal/wiki/` (a separate private repo, cloned next to this
one). Link to them; never copy their content into this repo.
