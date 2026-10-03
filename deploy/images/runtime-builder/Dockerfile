# syntax=docker/dockerfile:1.7
# Supply an operator-approved, digest-pinned Docker CLI image. This recipe
# contains no daemon, credentials, package publisher, or controller state.
ARG DOCKER_CLIENT_IMAGE
FROM ${DOCKER_CLIENT_IMAGE} AS docker-client
FROM ghcr.io/ayni-oss/ayni-env:0.14.4-debian@sha256:bc2bdcd92302482339d57b0eb05146c6a60dc97f82312227fb6e10b89f39c26b
USER root
COPY --from=docker-client /usr/local/bin/docker /usr/local/bin/docker
USER 10001
