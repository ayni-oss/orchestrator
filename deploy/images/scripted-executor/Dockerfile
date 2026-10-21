# syntax=docker/dockerfile:1.7
FROM rust:1.94.0-bookworm AS builder

WORKDIR /workspace
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY backend ./backend
COPY proto ./proto
RUN --mount=type=cache,id=ayni-orchestrator-executor-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ayni-orchestrator-executor-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=ayni-orchestrator-executor-target,target=/workspace/target \
    cargo build --locked --release --package ayni-orchestrator-scripted-executor \
      --bin ayni-orchestrator-scripted-executor \
    && install --mode=0755 target/release/ayni-orchestrator-scripted-executor \
      /tmp/ayni-orchestrator-scripted-executor

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --no-install-recommends --yes ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home orchestrator
USER 10001
COPY --from=builder /tmp/ayni-orchestrator-scripted-executor \
  /usr/local/bin/ayni-orchestrator-scripted-executor
ENTRYPOINT ["/usr/local/bin/ayni-orchestrator-scripted-executor"]
