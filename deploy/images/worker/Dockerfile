# syntax=docker/dockerfile:1.7
ARG AYNI_EXECUTOR_IMAGE=ghcr.io/ayni-oss/ayni-env:0.14.5-debian@sha256:d8818c4638e51d9d85ff09e050c035e3e6e548b38acc9b349de2a05c2f6253c8
FROM ${AYNI_EXECUTOR_IMAGE} AS ayni-cli
FROM rust:1.94.0-bookworm AS builder
WORKDIR /workspace
ARG TARGETARCH
RUN apt-get update && apt-get install --yes --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY backend ./backend
RUN --mount=type=cache,id=ayni-orchestrator-worker-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ayni-orchestrator-worker-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=ayni-orchestrator-worker-target,target=/workspace/target \
    case "$TARGETARCH" in \
      amd64) rust_target=x86_64-unknown-linux-musl ;; \
      arm64) rust_target=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported worker architecture" >&2; exit 1 ;; \
    esac \
    && rustup target add "$rust_target" \
    && cargo build --locked --release --target "$rust_target" --package ayni-orchestrator-worker --bin ayni-orchestrator-worker \
    && mkdir /tmp/worker-binaries \
    && install --mode=0755 "target/$rust_target/release/ayni-orchestrator-worker" /tmp/worker-binaries/

FROM debian:bookworm-slim
ARG TARGETARCH
RUN apt-get update && apt-get install --yes --no-install-recommends git ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
# Match the repository's pinned lock resolver. Checksums are from the immutable
# https://github.com/jdx/mise/releases/tag/v2026.6.14 release assets.
RUN case "$TARGETARCH" in \
      amd64) mise_arch=x64; mise_sha=c5bb4546ba2d5154e9c8236e2774bd8289b64c409330ed41cb6d6b8ebc31fb56 ;; \
      arm64) mise_arch=arm64; mise_sha=6d71ba01f20bb7cc7bfffac5214e9e788a08fa517c075eb955a5b31dca667ca7 ;; \
      *) echo "unsupported builder architecture" >&2; exit 1 ;; \
    esac \
    && curl --fail --location --retry 3 --output /tmp/mise.tar.gz \
       "https://github.com/jdx/mise/releases/download/v2026.6.14/mise-v2026.6.14-linux-${mise_arch}.tar.gz" \
    && printf '%s  /tmp/mise.tar.gz\n' "$mise_sha" | sha256sum --check - \
    && tar --extract --gzip --file /tmp/mise.tar.gz --directory /usr/local/bin --strip-components=2 mise/bin/mise \
    && rm /tmp/mise.tar.gz \
    && mise --version
COPY --from=ayni-cli /usr/local/bin/ayni /usr/local/bin/ayni
COPY --from=builder /tmp/worker-binaries/ /usr/local/bin/
RUN if ! getent group 10001 >/dev/null; then groupadd --gid 10001 ayni-worker; fi \
    && if ! getent passwd 10001 >/dev/null; then useradd --uid 10001 --gid 10001 --create-home ayni-worker; fi
USER 10001
ENTRYPOINT ["/usr/local/bin/ayni-orchestrator-worker"]
