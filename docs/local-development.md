# Local development

The supported topology uses Docker Desktop Kubernetes with explicit
`docker-desktop` context, a trusted maintainer team and loopback access.
`dev/ctl deploy` builds and digest-pins the worker and installs the local umbrella
chart at `dev/charts/ayni-orchestrator-local`. Production uses the reusable
chart with strict platform scheduling; local emulation is explicitly configured.

`dev/ctl hot-reload` retains the local Helm release and starts Tilt for server
and web iteration. Dependency, generated-binding, chart and credential changes
require restarting the deployment inputs. `dev/ctl` never falls back to another
cluster or prints credentials.

Admission freezes runtime/source identities and execution limits. Runtime
configuration defaults to 24 total coding turns and 3 quality repairs, with
repairs bounded to 0–10. Workers attest before checkout, commit coherent work,
run mandatory full checks, repair within budget, bind final evidence and deliver
truthful drafts. Control-plane collection is idempotent and cleanup follows
durable collection.

For a breaking deployment, inventory the exact application database volume,
worker-output PVCs, Jobs, ConfigMaps and attempt Secrets before stopping old
workers and resetting application state. Older database schemas are rejected
without conversion. Preserve source repositories, runtime images, unrelated
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
