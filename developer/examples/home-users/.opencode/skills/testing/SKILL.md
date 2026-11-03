---
name: testing
description: Use when writing or running tests, or when deciding whether a test needs real infrastructure.
---

# Testing

- `just test` — fast suite: unit + feature tests (axum-test, in-process) plus doctests. No infrastructure.
- `just test-pact` — contract tests, both sides. Needs Postgres.
- `just test-acceptance` — real Postgres/Valkey/RabbitMQ through the compose `test` profile, once. `just acceptance` leaves that stack idle for focused exec runs.
- `just check` is the pre-finish gate; `just coverage` writes LCOV.
