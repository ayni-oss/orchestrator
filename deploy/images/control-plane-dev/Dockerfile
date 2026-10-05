# syntax=docker/dockerfile:1.7
# Development-only image. The production image remains in ../control-plane.
FROM rust:1.94.0-bookworm

WORKDIR /opt/source
RUN useradd --system --uid 10001 --create-home orchestrator
COPY --chown=10001:10001 Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY --chown=10001:10001 backend ./backend
COPY --chown=10001:10001 proto ./proto
# WORKDIR was created as root. Cargo creates its target directory alongside the
# manifests, so ownership of the directory itself must match the build user.
RUN chown --recursive 10001:10001 /opt/source
USER 10001
# Compile once into the image. The local Helm overlay copies this seeded
# workspace to an emptyDir, where Tilt can safely sync and rebuild source.
RUN cargo build --locked --package ayni-orchestrator-control-plane

WORKDIR /workspace
EXPOSE 8080 50051 50052
CMD ["/workspace/target/debug/ayni-orchestrator-control-plane"]
