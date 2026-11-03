# Logs on stdout; OTLP optional and env-gated

Logging is JSON on stdout only — identical in every environment, with no in-app shipping; whatever listens (a collector, `kubectl logs`, anything later) stays external and swappable. `REQUEST_LOG_LEVEL` and `APP_LOG_LEVEL` are independent per-layer filters (renamed from `CUSTOM_LOG_LEVEL`); sensitive headers are never logged.

Distributed traces and RED metrics cannot ride text logs. `OTEL_EXPORTER_OTLP_ENDPOINT` is a reserved, documented hook — the OTLP exporter is deliberately deferred until OpenTelemetry Rust stabilizes (map Fog; the crate set is pre-evaluated in `docs/research/rust-ecosystem-2026-10.md`), so nothing reads the variable yet. When the exporter lands it will activate only when the variable is set: unset means nothing leaves the process. OTel Rust is 0.x with traces in Beta; versions are pinned exactly and bumped deliberately.
