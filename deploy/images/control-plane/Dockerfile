# syntax=docker/dockerfile:1.7
FROM rust:1.94.0-bookworm AS builder

WORKDIR /workspace
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY backend ./backend
COPY proto ./proto
RUN --mount=type=cache,id=ayni-orchestrator-control-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ayni-orchestrator-control-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=ayni-orchestrator-control-target,target=/workspace/target \
    cargo build --locked --release --package ayni-orchestrator-control-plane \
    && install --mode=0755 target/release/ayni-orchestrator-control-plane /tmp/ayni-orchestrator-control-plane

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --no-install-recommends --yes ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --create-home orchestrator
USER 10001
COPY --from=builder /tmp/ayni-orchestrator-control-plane /usr/local/bin/ayni-orchestrator-control-plane
EXPOSE 8080 50051 50052
ENTRYPOINT ["/usr/local/bin/ayni-orchestrator-control-plane"]
