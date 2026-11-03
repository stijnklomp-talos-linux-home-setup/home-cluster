# HTTP stack: tokio + axum + tower-http

The service app runs on tokio with axum (0.8) and tower-http layers — request IDs, tracing, sensitive-header redaction, compression, body limits, timeouts — with CORS present but off, matching the Fastify template. Shutdown on SIGTERM/SIGINT is real: stop accepting, drain in-flight requests under a timeout, cancel consumers, close database/cache/AMQP connections.

Chosen because axum is the de-facto default for new Rust services (tower ecosystem, tokio alignment, dense examples agents can pattern-match), and typed extractors/errors keep agent iterations cheap.
