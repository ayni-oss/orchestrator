# syntax=docker/dockerfile:1.7
FROM rust:1.94.0-bookworm AS builder

WORKDIR /workspace
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY backend ./backend
COPY proto ./proto
COPY apps/console ./apps/console
COPY dev/fixtures/task-source ./dev/fixtures/task-source
COPY dev/fixtures/task-quality ./dev/fixtures/task-quality
RUN --mount=type=cache,id=ayni-orchestrator-control-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ayni-orchestrator-control-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=ayni-orchestrator-control-target,target=/workspace/target \
    cargo build --locked --release --package ayni-orchestrator-control-plane \
      --bin ayni-orchestrator-control-plane --bin workflow-migrate --bin workflow-diagnostic --bin workflow-acceptance \
    && install --mode=0755 target/release/ayni-orchestrator-control-plane /tmp/ayni-orchestrator-control-plane \
    && install --mode=0755 target/release/workflow-migrate /tmp/workflow-migrate \
    && install --mode=0755 target/release/workflow-diagnostic /tmp/workflow-diagnostic \
    && install --mode=0755 target/release/workflow-acceptance /tmp/workflow-acceptance

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --no-install-recommends --yes ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home orchestrator
USER 10001
COPY --from=builder /tmp/ayni-orchestrator-control-plane /usr/local/bin/ayni-orchestrator-control-plane
COPY --from=builder /tmp/workflow-migrate /usr/local/bin/workflow-migrate
COPY --from=builder /tmp/workflow-diagnostic /usr/local/bin/workflow-diagnostic
COPY --from=builder /tmp/workflow-acceptance /usr/local/bin/workflow-acceptance
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/ayni-orchestrator-control-plane"]
