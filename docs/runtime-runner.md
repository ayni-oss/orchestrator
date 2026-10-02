# Shared immutable runtime runner

The shared runner accepts a controller-admitted OCI reference only as
`repository@sha256:<64 lowercase hex>`. A tag is discovery metadata and is never
passed to a coding Job. The controller freezes that exact reference with the
task/attempt snapshot before scheduling; node-local caching does not change its
identity.

## v1 contract

A supported runtime has one of `linux/amd64` or `linux/arm64`, an explicit
bounded launcher argv, a verified non-root effective user, a writable
`/workspace` checkout, and bounded scratch storage. Kubernetes supplies that
argv as `args`, never `command`: the image entrypoint is retained and is
responsible for any required preparation before it invokes the mounted launcher.
The launcher is mounted from an attempt-scoped ConfigMap by the job template; a
runtime image does not contain Orchestrator credentials or implementation.
The coding process runs as an unprivileged user with a read-only root filesystem,
no Kubernetes service-account token, no host socket, no controller database, and
no other task's volumes. Network policy and resource limits are supplied by the
job template, not the image.

Candidate output is a bounded bundle/commit protocol written at
`/workspace/.orchestrator/output` on an attempt-owned volume. Model, GitHub,
and registry credentials are task-scoped mounts or
process environment only: they are never image layers, command-line arguments,
or retained logs. Unsupported platform, mutable image reference, invalid argv,
or an unavailable launcher is an actionable unsupported-runtime outcome before
model work begins.

The runner renders a deterministic `batch/v1` Job before any Kubernetes mutation:
its name is derived from the attempt identity, `backoffLimit` is zero, the deadline
is bounded, a completed Job is retained briefly for diagnosis, and a retry gets a
new attempt identity. The Job uses `RuntimeDefault` seccomp, drops all capabilities,
runs as the dedicated agent UID/GID, mounts only workspace/scratch/output volumes,
and disables service-account token mounting. Its only writable volumes are
workspace, scratch, and candidate output; the launcher mount is read-only. An
optional pull-secret *name* is
attached only after controller-side creation of a short-lived attempt credential;
the secret material is never part of this contract or the rendered diagnostics.

Ayni lock interpretation, environment building, certification, and managed
validation are profile concerns. The shared runner deliberately contains none of
them.
