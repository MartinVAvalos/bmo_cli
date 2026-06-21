# ---- Build stage ----
# `rust:1-...` tracks the latest stable 1.x toolchain. Combined with --locked
# below, the pinned versions in Cargo.lock are what actually get built.
FROM rust:1-slim-bookworm AS builder

WORKDIR /app

# Copy manifests and sources, then build the optimized binary.
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

# ---- Runtime stage ----
# Slim image with just the compiled binary. Same Debian release as the builder,
# so the dynamically linked glibc matches.
FROM debian:bookworm-slim

# /work is where you mount the project you want to capture (see the Makefile).
WORKDIR /work

COPY --from=builder /app/target/release/bmo /usr/local/bin/bmo

# Args pass straight through, so `docker run <image> -last` / `-help` work.
ENTRYPOINT ["bmo"]