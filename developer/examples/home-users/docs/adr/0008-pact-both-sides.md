# Pact on both sides, examples shipped, no broker

A service can be a Pact consumer, a provider, or both. Consumer tests generate pact files with `pact_consumer`; provider verification runs in-process against checked-in `pacts/*.json` with `pact_verifier`, including a provider-state handler map. Because the Rust consumer DSL ships thin documentation, the template includes a working example of each side — the examples are the documentation. `just test-pact` runs the suite.

No broker is configured; the flow leaves a documented hook for publishing verification results once one exists. Pact files are checked into the repository, mirroring the current fragment-composer practice.
