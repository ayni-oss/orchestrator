# Decision 0012: in-container task validation

> Historical record: task-pipeline and quality/publication ownership in this decision are superseded by the [worker-owned architecture](../architecture.md). They do not require compatibility code.
- Status: accepted
- Date: 2026-10-06

## Decision

Ayni validation is a first-class coding-worker capability. The model can run
the pinned full check with `ayni_validate` while editing and must use
`finalize_candidate` to run the final check and freeze the resulting candidate
atomically. The controller projects only the bounded final result for that
candidate into a marker-owned PR-description section.

The GitHub Actions managed-assessment execution path is no longer reconciled or
installed for new repositories. Historical review/evidence records remain
readable during the schema transition, but do not govern new task publication.

## Consequences

Task validation is not an independent fresh-checkout assessment. Failed,
unavailable, and missing results still publish truthful drafts. Certified image
admission, immutable runtime identity, policy/lock protections, checkpoints,
and deterministic candidate publication remain unchanged.
