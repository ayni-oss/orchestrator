# syntax=docker/dockerfile:1.7
# Supply operator-approved, digest-pinned Docker CLI and Debian Ayni executor
# images. Git and CA certificates support frozen source checkout; Buildx supports
# Ayni environment preparation. This recipe contains no daemon or credentials.
ARG DOCKER_CLIENT_IMAGE=docker.io/library/docker:28.3.3-cli@sha256:0135662b510037ea581d99c2e5929c5e01185139c0b86986a418bd4da0b98a44
ARG AYNI_EXECUTOR_IMAGE=ghcr.io/ayni-oss/ayni-env:0.14.5-debian@sha256:d8818c4638e51d9d85ff09e050c035e3e6e548b38acc9b349de2a05c2f6253c8
FROM ${DOCKER_CLIENT_IMAGE} AS docker-client
FROM ${AYNI_EXECUTOR_IMAGE}
ARG TARGETARCH
USER root
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
COPY --from=docker-client /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker-client /usr/local/libexec/docker/cli-plugins/docker-buildx /usr/local/libexec/docker/cli-plugins/docker-buildx
LABEL org.opencontainers.image.source="https://github.com/ayni-oss/orchestrator" \
      org.opencontainers.image.version="0.14.5"
ENV HOME=/scratch TMPDIR=/scratch XDG_CACHE_HOME=/scratch/cache
RUN docker --version && docker buildx version && ayni --version
USER 10001
# Kubernetes supplies the trusted launcher as the command arguments.
ENTRYPOINT []
CMD ["/bin/sh"]
