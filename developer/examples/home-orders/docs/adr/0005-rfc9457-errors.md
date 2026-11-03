# One error contract: RFC 9457 problem+json

Every non-2xx response — validation, auth, not-found, internal, readiness — is `application/problem+json` generated from one shared `Problem` type, with validation details in an `errors` extension field. This replaces the Fastify template's three inconsistent body shapes (`{error}`, `{message}`, `{error: "Service Unavailable"}`).

Deliberate divergence: only services built from this template speak problem+json; nothing in the existing fleet needs to consume it.
