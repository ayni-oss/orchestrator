# syntax=docker/dockerfile:1.7
# Supply an operator-approved, digest-pinned Docker CLI image. This recipe
# contains no daemon, credentials, package publisher, or controller state.
ARG DOCKER_CLIENT_IMAGE
FROM ${DOCKER_CLIENT_IMAGE} AS docker-client
FROM ghcr.io/ayni-oss/ayni-env:0.14.2-debian@sha256:6b92c65b4592b043d58d6d4b6dc1b4fcf6c15ed8f6db59e9f3dd5dbe4dbbaba5
USER root
COPY --from=docker-client /usr/local/bin/docker /usr/local/bin/docker
USER 10001
