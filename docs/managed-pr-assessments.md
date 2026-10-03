# Managed PR assessments

Repository automation version `ayni-managed-v3` installs the environment build and two assessment workflows.
`Ayni assessment request` observes opened, reopened, synchronize, and
ready-for-review events. It executes no checkout or candidate program.
`Ayni Managed assessment` runs from the default branch on `workflow_run`, with
concurrency scoped to repository and PR. It runs the operator-selected collector
from an immutable private worker image. Neither workflow uses
`pull_request_target`.

## Configuration

The profile must already have an installed v3 automation and an admitted signed
Linux/amd64 environment. Add this operator-owned configuration to its existing
`runtime.profiles` entry:

```yaml
assessment:
  producer_image: ghcr.io/OWNER/PRIVATE-WORKER@sha256:FULL_MANIFEST_DIGEST
  publication_policy: DRAFT_ONLY
```

The collector is the `ayni-orchestrator-worker` binary built by the existing
private worker Dockerfile. Its image must contain the Linux/amd64 binary for
the Ubuntu GitHub runner. Package publication remains a separate operator
operation. The workflow must have package-read access to its own approved
environment and collector packages; it has no package-write or signing key.

Configure these **non-secret** Actions variables from the admitted profile:

| Variable | Value |
| --- | --- |
| `ASSESSMENT_PRODUCER_IMAGE` | Same immutable image as `producer_image` |
| `ASSESSMENT_PROFILE_REVISION` | Immutable Orchestrator profile revision ID |
| `ASSESSMENT_IMAGE_REPOSITORY` | Approved GHCR environment repository |
| `ASSESSMENT_RUNTIME_DIGEST` | Independently admitted `sha256:` OCI manifest digest |
| `ASSESSMENT_LOCK_FINGERPRINT` | Full committed `sha256:` lock fingerprint |
| `ASSESSMENT_CONTRACT_DIGEST` | SHA-256 of the approved `.ayni.toml` bytes |

The environment must also be discoverable as `lock-<full fingerprint without
sha256:>`. Discovery is compared with the configured immutable digest before
launch. Updating a profile requires updating its variables; mismatches fail
closed. A lock or policy change in a PR is unsupported, including an attempt to
replace the approved signing policy. Fork assessments are unsupported.

## Execution and evidence boundary

The collector obtains repository/PR/head/base/run identities through GitHub and
checks out the exact SHA using an ephemeral fetch header. Credentials are not
persisted. Candidate code executes only in the digest-pinned environment with
its original entrypoint, non-root identity, no capabilities, no network,
read-only root, resource limits, and bounded scratch. It receives no GitHub
token, Docker socket, controller state, signing seed, or registry credentials.
Tracked files and Git metadata have read-only mounts; source ancestor mounts
prevent renaming a directory around a file mount. Generated dependency state
can be prepared in the writable checkout. Tracked sources are measured before
and after execution. Symlink sources, oversized checkouts and unignored Ayni
generated state fail closed.

The trusted Ayni binary runs `check --output json`. Its original serialized
signals are captured through the container stdout stream into collector-owned
memory. After stopping the container, the collector requires byte equality with
the original `signals.json`, validates completion and signed runtime provenance,
and retains those original bytes. Additional stdout or candidate changes to the
artifact cannot substitute for Ayni's final serialization. This boundary relies
on the container PID namespace, default seccomp policy and disabled ptrace
capabilities; it does not treat arbitrary candidate-generated JSON as evidence.
Ayni performs certificate/content verification. The platform does not implement
another certificate verifier and never creates `execution.json`.

`manifest.json` uses `assessment-v1` and binds the repository, PR, exact head and
base, profile revision, workflow ID/SHA/run/attempt, collector digest, observed
OCI digest, contract, lock, Ayni version, platform, before/after source hashes,
exit outcome, and each retained artifact digest. The allowed files are
`manifest.json`, `signals.json`, and `runtime.json`; the total extracted bound is
10 MiB. Runs that cannot produce evidence cannot authorize readiness.

The importer uses a repository-scoped, short-lived App token with contents,
Actions and checks read permissions and PR write permission. It compares the
workflow files at GitHub's independently observed execution SHA with its own
templates, verifies the artifact archive digest and bounded contents, checks
the configured collector and previously admitted runtime identities, and then
imports one immutable assessment per repository/run/attempt. Evidence is stored
next to the controller database under `assessments/<assessment identity>/`;
that directory is never mounted into agent containers. A transaction stores
assessment/evidence metadata and review events together. Identical imports are
idempotent; conflicting bytes fail. Retained evidence can be reused after the
Actions artifact expires. The reconciler observes the latest 100 completed
assessment runs per profile; operators must reconcile before a larger backlog
accumulates. Older missed runs require a fresh workflow attempt.

## Projections and readiness

The collector owns the stable exact-head check named `Ayni Managed assessment`
and one `github-actions[bot]` comment delimited by
`<!-- orchestrator-assessment:v1 -->` and its closing marker. It reads before
creating, replaces only its marked block, preserves surrounding human text,
and checks the current PR head before projecting. Raw tool output, paths, and
candidate URLs are excluded. Detailed original findings remain in private
evidence. A damaged marker or ambiguous ownership fails closed. Comments and
checks never replace retained evidence.

All newly published Orchestrator PRs start as drafts. The existing publication
request's `draft` field is retained for wire compatibility but cannot bypass
the assessment gate. `DRAFT_ONLY` is the default. An operator may explicitly
select `READY_AFTER_SUCCESSFUL_ASSESSMENT`; candidate policy files cannot select
it. Human PRs receive assessments but remain outside automatic readiness.

The controller requires an open draft on the published candidate's exact SHA,
the owned branch/repository and expected base, current successful evidence, and
the latest stable check's matching successful workflow identity. It records
intent before calling GitHub, rehashes retained evidence and reads provider
state immediately before transition. An uncertain response is reconciled by
observing readiness rather than blindly issuing another transition; its source
is reported as `provider-observed` when the actor cannot be proven. Manual
readiness is not reversed. A subsequent head invalidates authorization and
surfaces `obsolete`; no automatic re-drafting occurs. GitHub's readiness mutation
has no atomic expected-head precondition, so a provider-side race after the final
read cannot be excluded; the next observation surfaces the changed head.

The v1 review API and `/reviews` BFF page expose history, exact-head outcomes,
bounded evidence identities, and readiness policy/action/source/diagnostic.
The clean schema advances from 1 to 2 to allow human PRs without a fabricated
agent candidate. Existing clean state is retained; retired pilot state remains
unsupported.

## Validation boundary

Contract, persistence, workflow-template and presentation tests are local
evidence only. Full acceptance additionally requires a signed sandbox image,
the approved collector package, configured Actions variables/App access, and a
two-head sandbox run that proves one updated comment and immutable history.
Unsigned images are rejected for authoritative assessment and automatic
readiness. They can still be used for ordinary development feedback.
