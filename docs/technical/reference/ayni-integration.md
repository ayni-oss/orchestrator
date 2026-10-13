# Ayni integration contract

The worker consumes Ayni's supported CLI and artifact contract. Managed runtime
identity includes the committed lock and pinned image digest; admission verifies
both before coding.

Focused feedback uses `ayni verify <signal>` and its separate artifact at
`.ayni/verify/last/signals.json`. Mandatory quality uses `ayni check` and the full
artifact at `.ayni/last/signals.json`. Typed selectors support language, root,
file, package and name where the adapter allows them; Ayni validates adapter
capabilities. Process errors and malformed or stale evidence remain explicit
unavailable outcomes.

The final full check must cover the final committed source. Completion counts,
signals, bounded findings, artifact digest, source fingerprint and final commit
are retained together and disclosed in worker-created drafts. Focused evidence
cannot satisfy the full gate. The runtime must pin a CLI release that supports
these commands and provenance checks; runtime-version changes require actual
managed-topology verification.

Environment build uses `ayni env build --executor-image <approved digest> --tag <candidate>` with selected trusted signing identity. Certification is performed by
that supported build contract after dependencies are prepared; an unsigned result
cannot become Ready. Separate publication pushes the certified candidate and
separate verification consumes Ayni’s signed metadata and fresh managed signals.
No orchestrator implementation interprets adapter policy or regenerates a lock
without the supported CLI. Environment certification and task code quality remain
separate outcomes. Orchestrator development CI invokes `ayni check` on the host with pinned quality tools;
it asserts `execution_mode: host` and does not build repository environment images.


The official builder identity is resolved through the [supported builder catalog](repository-builders.md).
Builder provenance, environment certification, and task evidence have different subjects; see [assurance boundaries](../architecture/assurance.md).
The [environment-change flow](../guides/environment-changes.md) is planned work and does not relax current task lock protection.
