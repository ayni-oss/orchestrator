# Managed runtime execution

The managed profile binds a connected repository, committed lock, verified
runtime artifact and digest. Reusable App credentials stay inside the connector
boundary. Each worker receives short-lived repository-scoped credentials for
checkout, push and draft creation. The control plane refreshes attempt
credentials while the worker is active.

The worker attests its runtime before checkout, verifies source and lock, codes,
runs mandatory full checks, repairs within frozen limits, binds final evidence
and publishes a truthful draft. The control plane only schedules, observes,
collects and cleans up task execution. Environment build Jobs remain independent
of task execution and retain their narrowly scoped build authority.

The Helm runtime configuration exposes `codingTurns: 24`,
`qualityRepairLimit: 3`, concurrency and runtime policy. Repair limits range from
0 to 10. Changes to deployment defaults affect newly admitted tasks; frozen
attempt limits remain unchanged after restart.

Local execution uses the Docker Desktop umbrella chart described in
[local development](local-development.md). Production retains platform node
selection, digest pinning and resource limits; emulation is a local policy.

## Worker sandbox

The Linux worker requires fully enforced Landlock ABI 6 (kernel 6.12 or newer).
Both agent shell commands and Ayni quality execution run inside the same
unprivileged sandbox because repository tests are untrusted code. System and
toolchain paths are read-only; the workspace and disposable scratch paths are
writable. Worker output, repository/model credentials, frozen requests and the
launcher remain outside that authority. Signal and abstract Unix-socket scopes
prevent sandboxed children from signaling the parent or reaching its abstract
sockets. Unsupported or partial enforcement fails closed: there is no
unrestricted fallback. This needs no privileged container, added capabilities,
or special Docker/Kubernetes privilege.
