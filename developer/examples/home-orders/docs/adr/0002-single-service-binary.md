# One binary per service; no runtime modes

The Fastify template ran one codebase in `api`/`worker` modes via `DEPLOYMENT_MODE` — a workaround for Node's worker-thread CPU model. The Rust template ships a single binary: the service app.

The AMQP consumer/DLQ code is a tested library module but is not started by default; a service that wants a dedicated consumer deployment adds a small, documented `src/bin/worker.rs` (or wires the consumer in-process). Consumer deployments are a per-service decision, not a template mode.
