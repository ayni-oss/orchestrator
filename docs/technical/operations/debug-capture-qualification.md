# Debug capture qualification

Delivery remains in progress. The implementation and these checks do not yet
establish the full four-slice acceptance contract. See the
[architecture contract](../architecture/execution-capture.md).

## Local evidence, 11 November 2026

The explicitly selected `docker-desktop` context and isolated
`ayni-orchestrator-capture` namespace exercised the real control plane,
PostgreSQL, collector, loopback gateway and frontend. Retained stores were not
reset or deleted.

The deployed browser/Kubernetes fixture passed:

- Regular, init and sidecar source registration; failed-to-start registration.
- Exclusion of an unrelated pod without managed ownership labels.
- Repeated identical lines, binary bytes and original-byte download.
- Retained output after deleting the fixture pod.
- Frontend history, follow/pause, inert HTML and terminal controls.
- Debug-off direct reads, streams and downloads returning 403, page access
  returning 404, and hidden navigation. Debug mode was then restored.

The qualification scripts are `scripts/ci/check-debug-logs-deployed.mjs` and
`scripts/ci/check-debug-recovery-deployed.mjs`. Set
`AYNI_ORCHESTRATOR_NAMESPACE` to the dedicated acceptance namespace, optionally
`AYNI_DEBUG_GATEWAY` and `AYNI_DEBUG_FRONTEND` to its loopback listeners, and run
with the repository-pinned Node version. The recovery script has not yet passed
against the final deployment; it covers crash-loop inventory, OOM, forced deletion
and collector restart. These scripts delete only their own disposable fixture
pods, except for the intentional collector restart, which preserves its PVC.

The last successfully deployed images were:

- `local/ayni-orchestrator-control-plane:git-98d1cdf9b018-633f755b8c94`
- `local/ayni-orchestrator-conversation-supervisor:git-98d1cdf9b018-da9297b31d8b`

The working tree contains subsequent source-status, loss-marker and publication
correlation fixes. Those images are therefore not final-source qualification.

Host validation passed formatting, Clippy, 247 Rust tests (98 environment-dependent
tests skipped), 13 frontend tests, production frontend build, generated protocol
checks, Helm/topology checks and documentation validation. Focused PostgreSQL
replay/retention tests passed before the last source-status query changes; the
final rerun failed while starting the disposable database. The Svelte diagnostic
runner also emitted its existing TypeScript compatibility error, so its zero-error
summary is not treated as a completed typecheck.

## Remaining gates

Run the final pinned Rust, protocol, frontend, chart and PostgreSQL checks on
frozen inputs, rebuild both images, then repeat the deployed fixtures. Qualify
all real runtime/task/verification/publication paths, nested-container cleanup,
cancellation and abandoned-spool recovery, worker/control-plane/collector outages,
rotation, eviction and storage exhaustion. Preserve verifier parsing/isolation and
stale-generation rejection while doing so. Existing task-quality acceptance has a
repository-declared retired-browser-coverage block; replace that path before
claiming complete product coverage.

A managed Ayni environment was built from the generated lock. Its frozen-source
check reached test/size/complexity/dependency phases, then failed to persist the
final artifact because Docker storage was exhausted. Docker reported its metadata
filesystem read-only. No successful final managed report exists. The repository's
8 GiB free-space guard also blocked the final image rebuild. Restore storage
without deleting retained deployment data before continuing qualification.

Signing was subsequently approved through the existing 1Password SSH agent.
The foundation, collector and frontend slices are signed and pushed as draft PRs
[#289](https://github.com/ayni-oss/orchestrator/pull/289),
[#290](https://github.com/ayni-oss/orchestrator/pull/290), and
[#291](https://github.com/ayni-oss/orchestrator/pull/291). This fourth slice adds
qualification gates and records outstanding coverage; it does not claim closure.
All PRs target `alpha` and must merge in order. Nothing was merged automatically.

Removing only regenerable Rust incremental caches recovered approximately 38 GiB
of host capacity. A Docker Desktop restart was explicitly approved to restore the
local test server; final managed and deployed qualification must still be rerun.
