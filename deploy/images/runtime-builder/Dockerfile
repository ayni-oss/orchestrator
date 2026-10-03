# syntax=docker/dockerfile:1.7
# Supply an operator-approved, digest-pinned Docker CLI image. This recipe
# contains no daemon, credentials, package publisher, or controller state.
ARG DOCKER_CLIENT_IMAGE
FROM ${DOCKER_CLIENT_IMAGE} AS docker-client
FROM ghcr.io/ayni-oss/ayni-env:0.14.5-debian@sha256:d8818c4638e51d9d85ff09e050c035e3e6e548b38acc9b349de2a05c2f6253c8
USER root
COPY --from=docker-client /usr/local/bin/docker /usr/local/bin/docker
USER 10001
