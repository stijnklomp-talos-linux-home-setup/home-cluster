# syntax=docker/dockerfile:1

# One image for the service app (ADR-0009): a cargo-chef dependency layer on
# the pinned Rust slim image, then a distroless runtime. `just docker-build`
# tags it home-users:local; the compose `local`/`publish` profiles run it;
# `just docker-publish` builds the amd64 + arm64 manifest from this file.
#
# The build is hermetic: `--locked` everywhere, SQLX_OFFLINE=true plus the
# committed `.sqlx` cache (no database at image build time) and no rustup
# channel sync.

FROM rust:1.99.1-slim-trixie AS chef
WORKDIR /app

# The image ships rustc/cargo/rust-std only, while the committed
# rust-toolchain.toml (channel 1.99.1) also asks for clippy/rustfmt, which only
# the dev loop uses. Pinning RUSTUP_TOOLCHAIN to the installed toolchain stops
# rustup from syncing the channel (and downloading components) on every cargo
# invocation; the version tracks `rust-toolchain.toml`.
ENV RUSTUP_TOOLCHAIN=1.99.1

# cargo-chef is pinned (research §8). aws-lc-sys — the rustls crypto backend
# sqlx and lapin resolve — compiles C with cmake; the image already ships gcc,
# libc6-dev and perl.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    apt-get update \
    && apt-get install -y --no-install-recommends cmake make \
    && rm -rf /var/lib/apt/lists/* \
    && cargo install --locked cargo-chef --version 0.1.78

# The recipe holds only manifests + Cargo.lock, so it is byte-stable across
# source-only edits; the cook layer below survives them.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Dependencies, cached: reruns only when the recipe changes. registry/git/
# target live in cache mounts, not in build layers.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo chef cook --release --locked --features graphql --bin home-users --bin migrate --recipe-path recipe.json

# Image builds never need a live database: the sqlx macros resolve against the
# committed `.sqlx` offline cache (the copied `.cargo/config.toml` says the
# same; setting it here keeps the build independent of that file).
ENV SQLX_OFFLINE=true
COPY . .
# The binary must land in an ordinary layer in the same RUN as the cache
# mounts — mounts are not layer content.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release --locked --features graphql --bin home-users --bin migrate \
    && mkdir -p /out/app \
    && cp target/release/home-users /out/app/home-users \
    && cp target/release/migrate /out/app/migrate

# Distroless: glibc + ca-certificates, no shell, non-root by default.
# `COPY` creates /app from a directory (not into a pre-made one), so the
# `--chown` sticks to /app itself: non-production startup writes the generated
# openapi.yaml next to the binary (ADR-0004) and must be able to.
FROM gcr.io/distroless/cc-debian13:nonroot
USER 65532:65532
COPY --from=builder --chown=65532:65532 /out/app /app
WORKDIR /app
ENV API_PORT=3000
EXPOSE 3000
ENTRYPOINT ["/app/home-users"]
