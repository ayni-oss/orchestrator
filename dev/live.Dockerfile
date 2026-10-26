# syntax=docker/dockerfile:1.7
# Development only: incremental Linux debug builds, no credentials or host mounts.
FROM rust:1.94.0-bookworm AS builder
WORKDIR /workspace
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY backend ./backend
COPY proto ./proto
COPY dev/fixtures/task-source ./dev/fixtures/task-source
COPY dev/fixtures/task-quality ./dev/fixtures/task-quality
ARG COMPONENT
RUN --mount=type=cache,id=ayni-live-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ayni-live-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=ayni-live-target,target=/workspace/target \
    set -eu; mkdir /output; \
    case "$COMPONENT" in \
      control-plane) package=ayni-orchestrator-control-plane; \
        bins='ayni-orchestrator-control-plane workflow-migrate workflow-diagnostic workflow-acceptance workflow-publish-task' ;; \
      conversation-supervisor) package=ayni-orchestrator-conversation-supervisor; \
        bins='ayni-orchestrator-conversation-supervisor ayni-task-worker ayni-task-supervisor ayni-task-publisher' ;; \
      *) exit 2 ;; \
    esac; \
    set --; for bin in $bins; do set -- "$@" --bin "$bin"; done; \
    CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=1 cargo build --locked -p "$package" "$@"; \
    for bin in $bins; do install -m 0755 "target/debug/$bin" "/output/$bin"; done
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install --no-install-recommends --yes ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home orchestrator
USER 10001
COPY --from=builder /output/ /usr/local/bin/
# Direct entrypoints preserve the session-worker arguments used by supervisors.
FROM runtime AS control-plane
ENTRYPOINT ["/usr/local/bin/ayni-orchestrator-control-plane"]
FROM runtime AS conversation-supervisor
ENTRYPOINT ["/usr/local/bin/ayni-orchestrator-conversation-supervisor"]
