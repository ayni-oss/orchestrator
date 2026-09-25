ARG DOCKER_CLI_IMAGE=docker:27.5.1-cli@sha256:851f91d241214e7c6db86513b270d58776379aacc5eb9c4a87e5b47115e3065c
FROM ${DOCKER_CLI_IMAGE} AS docker-cli

FROM rust:1.94.0-bookworm AS builder
WORKDIR /workspace
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo build --locked --release --package ayni-orchestrator-worker

FROM debian:bookworm-slim
ARG TARGETARCH
RUN apt-get update \
    && apt-get install --no-install-recommends --yes ca-certificates curl git \
    && rm -rf /var/lib/apt/lists/*
COPY --from=docker-cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/docker-buildx /usr/local/libexec/docker/cli-plugins/docker-buildx
COPY --from=builder /workspace/target/release/ayni-orchestrator-worker /usr/local/bin/ayni-orchestrator-worker
COPY --from=builder /workspace/target/release/development-agent /usr/local/bin/development-agent
COPY --from=builder /workspace/target/release/pr-agent /usr/local/bin/pr-agent
COPY --from=builder /workspace/target/release/prepare-repository /usr/local/bin/prepare-repository
COPY --from=builder /workspace/target/release/validate-candidate /usr/local/bin/validate-candidate
COPY --chown=10001:10001 --from=builder /usr/local/cargo /usr/local/cargo
COPY --chown=10001:10001 --from=builder /usr/local/rustup /usr/local/rustup
ENV PATH="/usr/local/cargo/bin:${PATH}"
ENV RUSTUP_HOME=/usr/local/rustup
RUN case "$TARGETARCH" in \
      amd64) archive=ayni-ayni-v0.13.0-x86_64-unknown-linux-gnu.tar.gz; checksum=d0348577b129d9dac115b9a0bec164a73207c0d6a4bfdeb22ce8899060080ff8 ;; \
      arm64) archive=ayni-ayni-v0.13.0-aarch64-unknown-linux-gnu.tar.gz; checksum=10219e359e9059637589dd10fee62c731a814717e3dfcc16cbe22cdee48ff581 ;; \
      *) echo "unsupported target architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && curl --fail --location --retry 3 --output /tmp/ayni.tar.gz "https://github.com/ayni-oss/ayni/releases/download/ayni-v0.13.0/$archive" \
    && echo "$checksum  /tmp/ayni.tar.gz" | sha256sum --check \
    && tar --extract --gzip --strip-components=1 --file /tmp/ayni.tar.gz --directory /tmp \
    && install --mode=0755 /tmp/ayni /usr/local/bin/ayni \
    && rm /tmp/ayni.tar.gz /tmp/ayni
RUN if ! getent group 10001 >/dev/null; then groupadd --gid 10001 ayni-worker; fi \
    && if ! getent passwd 10001 >/dev/null; then useradd --uid 10001 --gid 10001 --create-home ayni-worker; fi
USER 10001
ENTRYPOINT ["/usr/local/bin/ayni-orchestrator-worker"]
