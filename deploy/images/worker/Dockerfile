# syntax=docker/dockerfile:1.7
FROM rust:1.94.0-bookworm AS builder
WORKDIR /workspace
ARG TARGETARCH
RUN apt-get update && apt-get install --yes --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY backend ./backend
COPY deploy/images/runtime-builder/Dockerfile ./deploy/images/runtime-builder/Dockerfile
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
# Package the approved CLI independently of the managed executor registry.
ARG AYNI_VERSION=0.14.5
ARG AYNI_AMD64_SHA256=45d20c9ae833b0cd403a917b2e07667cb2a6a4130a71d2bf94691c1d651e0139
ARG AYNI_ARM64_SHA256=e57d65e87a6b7827d1534c21c3f88ee428e088991f0e7b04be28c1771fa54849
RUN case "$TARGETARCH" in \
      amd64) ayni_target=x86_64-unknown-linux-gnu; ayni_sha="$AYNI_AMD64_SHA256" ;; \
      arm64) ayni_target=aarch64-unknown-linux-gnu; ayni_sha="$AYNI_ARM64_SHA256" ;; \
      *) echo "unsupported worker architecture" >&2; exit 1 ;; \
    esac \
    && curl --fail --location --proto '=https' --tlsv1.2 --retry 3 --output /tmp/ayni.tar.gz \
       "https://github.com/ayni-oss/ayni/releases/download/ayni-v${AYNI_VERSION}/ayni-ayni-v${AYNI_VERSION}-${ayni_target}.tar.gz" \
    && printf '%s  /tmp/ayni.tar.gz\n' "$ayni_sha" | sha256sum --check - \
    && tar --extract --gzip --file /tmp/ayni.tar.gz --directory /usr/local/bin --strip-components=1 \
    && rm /tmp/ayni.tar.gz \
    && test "$(ayni --version)" = "ayni ${AYNI_VERSION}"
COPY --from=builder /tmp/worker-binaries/ /usr/local/bin/
RUN if ! getent group 10001 >/dev/null; then groupadd --gid 10001 ayni-worker; fi \
    && if ! getent passwd 10001 >/dev/null; then useradd --uid 10001 --gid 10001 --create-home ayni-worker; fi
USER 10001
ENTRYPOINT ["/usr/local/bin/ayni-orchestrator-worker"]
