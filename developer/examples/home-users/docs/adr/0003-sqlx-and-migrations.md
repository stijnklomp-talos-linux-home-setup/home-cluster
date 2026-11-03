# sqlx with offline cache; no ORM

Database access is sqlx with compile-time-checked SQL against a committed `.sqlx` offline cache (`SQLX_OFFLINE=true`), so builds and tests never require a live database; `cargo sqlx prepare --check` runs in `just check` to catch stale queries. Migrations are sqlx-cli migrations owned by the service (its own tracking table), and `just db-seed` populates dev data.

Chosen for agent-first development: compile-time errors replace runtime errors, and the offline cache keeps the database out of the edit/compile loop that an ORM's live-DB codegen would reintroduce. Fresh start: there is no Prisma or existing-service migration history to interop with.
