# syntax=docker/dockerfile:1.7
# Supply an operator-approved, digest-pinned Docker CLI image. This recipe
# contains no daemon, credentials, package publisher, or controller state.
ARG DOCKER_CLIENT_IMAGE
ARG AYNI_EXECUTOR_IMAGE
FROM ${DOCKER_CLIENT_IMAGE} AS docker-client
FROM ${AYNI_EXECUTOR_IMAGE}
USER root
COPY --from=docker-client /usr/local/bin/docker /usr/local/bin/docker
USER 10001
