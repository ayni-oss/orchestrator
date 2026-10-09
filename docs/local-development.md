# Local development

The supported topology uses Docker Desktop Kubernetes with explicit
`docker-desktop` context, a trusted maintainer team and loopback access.
`dev/ctl deploy` builds and digest-pins the worker and installs the local umbrella
chart at `dev/charts/ayni-orchestrator-local`. Production uses the reusable
chart with strict platform scheduling. Local managed runtime defaults to native
`linux/arm64` with emulation disabled; Intel hosts select `linux/amd64` explicitly.

`dev/ctl hot-reload` retains the local Helm release and starts Tilt for server
and web iteration. Dependency, generated-binding and chart changes require
restarting the deployment inputs. Connection Secret rotation reconciles without
restarting Tilt or the control plane. `dev/ctl` never falls back to another
cluster or prints credentials.

Admission freezes runtime/source identities and execution limits. Runtime
configuration defaults to 24 total coding turns and 3 quality repairs, with
repairs bounded to 0–10. Workers attest before checkout, commit coherent work,
run mandatory full checks, repair within budget, bind final evidence and deliver
truthful drafts. Control-plane collection is idempotent and cleanup follows
durable collection.

For a breaking deployment, inventory the exact application database volume,
worker-output PVCs, Jobs, ConfigMaps and attempt Secrets before stopping old
workers and resetting application state. SQLite schema version 8 migrates recognized versions 6/7; see operations before upgrade. Preserve source repositories, runtime images, unrelated
resources and existing PRs. Follow [operations](operations.md); broad namespace
purges are not the task-pipeline cutover procedure.

Run Rust formatting, Clippy, tests and compilation; protocol generation and
baseline checks; TypeScript/Svelte tests and builds; chart validation; and browser
checks. Fixture-backed UI checks do not replace actual worker-topology evidence.
Exercise managed runtime and publication against the authorized sandbox using
the explicit local context before claiming deployable behavior is validated.

The worker launcher is built as a static musl executable for the Docker Desktop
node architecture. Its mandatory kernel sandbox must run natively. Use a signed
managed environment matching the node architecture; the Ayni lock supports both
amd64 and arm64. Docker Desktop Kubernetes on Apple Silicon exposes Rosetta's
interpreter through a detached host mount that the sandbox cannot authorize, so
use a native arm64 environment there. Unsupported emulation fails admission.

Local builds check free space on the host volumes containing the repository and
home directory as well as inside Docker Desktop. The VM's virtual free space
does not guarantee that its sparse disk can grow on the host. Builds refuse to
start below the configured capacity threshold; cleanup remains an explicit
operator action and never implicitly removes persistent volumes.

The hot-reload control plane runs under a development-only process supervisor.
Tilt first synchronizes the current Rust source, protobuf definitions and Cargo
inputs, then compiles the binary. Later edits compile and signal only the child
process; they never terminate container PID 1. Restarting the container would
discard synchronized files and the new binary from its writable image layer.
Production images keep their normal direct entrypoint. To refresh an existing
supervised development container explicitly, run
`bash dev/scripts/sync-control-plane-source.sh`. This preserves application data
and credentials and uses only the `docker-desktop` context.


Repository environments follow the same automatic build/certify/publish/verify flow
locally. Ordinary component image packaging remains available for local deployment.
The local chart explicitly selects local-development disposable engines. Official
builders are pulled by resolved digest; no custom builder packaging or image seeding
is required. GHCR push and distinct pull-only credentials, a trusted certificate,
and a compatible committed policy/lock are prerequisites. Always use
`--context docker-desktop` for validation. Deploy cleaned values with `--reset-values`.

Exercise missing-image onboarding, verified reuse, explicit mismatch Rebuild,
uncertain push recovery, and task admission by immutable digest. Inspect workload
specifications for hostPath, privileged access, engine/signing/push credentials in
task or verification Jobs. Never delete retained task data to make validation pass.
