# Native arm64 profile. Inputs are static MUSL binaries built from this checkout.
FROM quay.io/skopeo/stable@sha256:2681eccb8b02c495b013c2669cbf7f41d5c0b0b0e644e7c69bd3ea9bcb20641f AS skopeo
FROM quay.io/podman/stable@sha256:f7a17ef911a2b9b866ada8a9426a07e29a6e4f8109ef8c0e0b7010c2541b4e49
USER root
COPY --from=skopeo /usr/bin/skopeo /usr/bin/skopeo
ARG AYNI_CLI_SHA256
COPY ayni /usr/local/bin/ayni
RUN test -n "$AYNI_CLI_SHA256" && echo "$AYNI_CLI_SHA256  /usr/local/bin/ayni" | sha256sum -c - && chmod 0555 /usr/local/bin/ayni
COPY umoci.linux.arm64 /usr/bin/umoci
RUN echo '5cfd17f2e7a4bcf9ed67ea1b955ca893d200349b9ce6a3d3707dba415f458a1f  /usr/bin/umoci' | sha256sum -c - && chmod 0555 /usr/bin/umoci
COPY runtime-worker /usr/local/bin/ayni-runtime-worker
COPY runtime-supervisor /usr/local/bin/ayni-runtime-supervisor
COPY runtime-verifier /usr/local/bin/ayni-runtime-verifier
RUN chmod 0555 /usr/local/bin/ayni-runtime-*
USER 1000:1000
ENTRYPOINT ["/usr/local/bin/ayni-runtime-worker"]
# Fail packaging before deployment if the immutable tool combination is incomplete.
RUN /usr/bin/git --version && /usr/bin/skopeo --version && /usr/bin/podman --version && /usr/bin/umoci --version && /usr/local/bin/ayni --version
