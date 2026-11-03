# AMQP: lapin, frozen semantics, hardened transport

The RabbitMQ wrapper's observable behavior is frozen and ported 1:1: exchanges `main` (alternate `nonRouted`, 24h TTL) and `deadLetter`; queue `task_queue` bound `task.#` (1h TTL, DLX `deadLetter`); declarations on both start paths; failed messages republished to `<queue>.failed` with error metadata, then always acked (no requeue). The client is lapin.

Three transport-level upgrades are deliberate: automatic reconnection, awaited publisher confirms (so "republish to DLQ, then ack" cannot silently lose messages), and a configurable `WORKER_PREFETCH` (default 1; previously effectively unlimited). Callers keep the existing convention: log and continue on publish failure.
