# Runtime preparation workload input

`source/` is a small Node workload repository retained as test input. It is not
an infrastructure or test runner. The Rust worker and verifier tests cover the
runtime contracts; `cargo xtask dev` builds pinned worker, verifier and executor
images for the canonical development environment.

The previous synthetic authority servers and separate-cluster qualification
launchers have been retired. Development uses `ayni-orchestrator-live-runtime`
as an isolation boundary inside the one environment. Docker Desktop remains
unqualified for NetworkPolicy enforcement. See
[local development](../../../docs/technical/operations/local-development.md).
