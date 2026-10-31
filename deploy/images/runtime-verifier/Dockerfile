# syntax=docker/dockerfile:1.7
FROM rust:1.94.0-alpine3.23 AS builder
RUN apk add --no-cache build-base musl-dev binutils git
WORKDIR /workspace
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY backend ./backend
COPY proto ./proto
COPY dev/fixtures/task-source ./dev/fixtures/task-source
COPY dev/fixtures/task-quality ./dev/fixtures/task-quality
RUN --mount=type=cache,id=ayni-runtime-verifier-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ayni-runtime-verifier-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=ayni-runtime-verifier-target,target=/workspace/target \
    set -eu; \
    verifier_target="$(rustc -vV | sed -n 's/^host: //p')"; \
    RUSTFLAGS='-C target-feature=+crt-static' cargo build --locked --release --target "$verifier_target" \
      --package ayni-orchestrator-runtime-verifier; \
    mkdir /output; \
    cp "target/$verifier_target/release/ayni-orchestrator-runtime-verifier" /output/ayni-runtime-verifier; \
    readelf --program-headers /output/ayni-runtime-verifier > /output/program-headers.txt; \
    readelf --dynamic /output/ayni-runtime-verifier > /output/dynamic.txt; \
    if grep -q INTERP /output/program-headers.txt; then exit 1; fi; \
    if grep -q '(NEEDED)' /output/dynamic.txt; then exit 1; fi

FROM scratch
USER 10001:10001
COPY --from=builder /output/ayni-runtime-verifier /ayni-runtime-verifier
ENTRYPOINT ["/ayni-runtime-verifier"]
