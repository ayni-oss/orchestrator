FROM rust:1.94.0-bookworm AS builder

WORKDIR /workspace
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo build --locked --release --package ayni-orchestrator-control-plane

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --no-install-recommends --yes ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --create-home orchestrator
USER 10001
COPY --from=builder /workspace/target/release/ayni-orchestrator-control-plane /usr/local/bin/ayni-orchestrator-control-plane
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/ayni-orchestrator-control-plane"]
