# Distroless images; compose profiles with distinct jobs

One multi-stage Dockerfile (cargo-chef + BuildKit cache) builds a distroless `cc-debian13:nonroot` image containing the service app. Multi-arch (amd64 + arm64) manifest builds are produced by `just docker-publish`.

Compose profiles: `infra` (Postgres/Valkey/RabbitMQ only), `dev` (source-mounted hot reload — the daily driver), `local` (build from the Dockerfile and run), `publish` (run a *published* registry image; no build), `test` (full stack + acceptance tests once, then exit), `acceptance` (full stack up and idle — run all tests, one test, or debug by hand). Kubernetes gets plain manifests (Deployment/Service/ConfigMap with liveness/readiness probes on the health endpoints); no CronJob example ships.
