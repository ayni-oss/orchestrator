# syntax=docker/dockerfile:1.7
FROM rust:1.94.0-bookworm AS builder
WORKDIR /workspace
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY backend ./backend
RUN --mount=type=cache,id=ayni-orchestrator-worker-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ayni-orchestrator-worker-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=ayni-orchestrator-worker-target,target=/workspace/target \
    cargo build --locked --release --package ayni-orchestrator-worker --bin ayni-orchestrator-worker \
    && mkdir /tmp/worker-binaries \
    && install --mode=0755 target/release/ayni-orchestrator-worker /tmp/worker-binaries/

FROM debian:bookworm-slim
RUN apt-get update && apt-get install --yes --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /tmp/worker-binaries/ /usr/local/bin/
RUN if ! getent group 10001 >/dev/null; then groupadd --gid 10001 ayni-worker; fi \
    && if ! getent passwd 10001 >/dev/null; then useradd --uid 10001 --gid 10001 --create-home ayni-worker; fi
USER 10001
ENTRYPOINT ["/usr/local/bin/ayni-orchestrator-worker"]
