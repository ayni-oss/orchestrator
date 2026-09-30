# Roadmap

Ayni Orchestrator requirements, priorities, and delivery status are maintained
in the private [organization Project](https://github.com/orgs/ayni-oss/projects/2)
and its linked [issues](https://github.com/ayni-oss/orchestrator/issues). An
issue is the source of truth for outcome and acceptance criteria; pull requests
close issues with a closing keyword.

## Current pre-public direction

The current delivery milestone is [#51](https://github.com/ayni-oss/orchestrator/issues/51):
one shared development foundation with **Ayni Managed** as the first complete
runtime profile and **Custom Runtime** as a later profile. The clean-v1
architecture is normative in [Decision 0007](docs/decisions/0007-clean-v1-runtime-profiles.md).
The earlier pilot API, database, preparation lifecycle, and validation model are
retired rather than current roadmap capabilities.

| Delivery concern | Issue(s) |
| --- | --- |
| Ayni OCI contract proof and profile architecture | [#52](https://github.com/ayni-oss/orchestrator/issues/52), [#53](https://github.com/ayni-oss/orchestrator/issues/53) |
| Clean API and durable reconciliation | [#54](https://github.com/ayni-oss/orchestrator/issues/54), [#55](https://github.com/ayni-oss/orchestrator/issues/55) |
| Digest-pinned execution and least privilege | [#56](https://github.com/ayni-oss/orchestrator/issues/56), [#57](https://github.com/ayni-oss/orchestrator/issues/57) |
| Setup, environment, and default-branch automation | [#58](https://github.com/ayni-oss/orchestrator/issues/58), [#59](https://github.com/ayni-oss/orchestrator/issues/59), [#60](https://github.com/ayni-oss/orchestrator/issues/60), [#61](https://github.com/ayni-oss/orchestrator/issues/61) |
| Exact-head assessment and readiness | [#62](https://github.com/ayni-oss/orchestrator/issues/62), [#63](https://github.com/ayni-oss/orchestrator/issues/63), [#64](https://github.com/ayni-oss/orchestrator/issues/64) |
| Operator UI and end-to-end operations | [#65](https://github.com/ayni-oss/orchestrator/issues/65), [#68](https://github.com/ayni-oss/orchestrator/issues/68) |
| Enabled follow-ups | [#66 Custom Runtime](https://github.com/ayni-oss/orchestrator/issues/66), [#67 artifact retention](https://github.com/ayni-oss/orchestrator/issues/67) |

## Planned implementation sequence

Issue acceptance criteria remain authoritative. The intended reviewable delivery
sequence is:

1. [#57](https://github.com/ayni-oss/orchestrator/issues/57) (least-privilege GitHub, GHCR, webhook, and certificate-signing boundaries) and [#56](https://github.com/ayni-oss/orchestrator/issues/56) (shared OCI runner) proceed independently.
2. [#58](https://github.com/ayni-oss/orchestrator/issues/58) installs the reviewed workflow templates after #57 establishes their permissions. [#59](https://github.com/ayni-oss/orchestrator/issues/59) can implement its setup, GHCR, immutable-digest, and no-op reuse paths in parallel, but its certified-publication completion waits for Ayni's public certificate contract and image generation ([ayni#33](https://github.com/ayni-oss/ayni/issues/33), [ayni#34](https://github.com/ayni-oss/ayni/issues/34)).
3. [#60](https://github.com/ayni-oss/orchestrator/issues/60) implements independent GitHub/GHCR reconciliation after #58/#59. It may deliver its state machine and digest/ownership checks before Ayni admission, but certificate-aware readiness waits for external-image admission ([ayni#35](https://github.com/ayni-oss/ayni/issues/35)).
4. [#61](https://github.com/ayni-oss/orchestrator/issues/61) consumes the frozen verified artifact for coding tasks once #56 and the applicable #60 readiness behavior are available.
5. [#62](https://github.com/ayni-oss/orchestrator/issues/62) and [#63](https://github.com/ayni-oss/orchestrator/issues/63) may share one PR for exact-head assessment production and verified import. Their certificate-backed managed-check completion waits for Ayni admission and certified runtime execution ([ayni#35](https://github.com/ayni-oss/ayni/issues/35), [ayni#36](https://github.com/ayni-oss/ayni/issues/36)); [#64](https://github.com/ayni-oss/orchestrator/issues/64) may join when its GitHub readiness transition remains reviewable.
6. Deliver [#65](https://github.com/ayni-oss/orchestrator/issues/65) incrementally with its backend capabilities, then close it when its full operator-workflow criteria are met.
7. [#68](https://github.com/ayni-oss/orchestrator/issues/68) is the final end-to-end operational proof and closure gate.

Ayni's certificate/admission work does not block #56, #57, #58, or the non-certificate portions of #59/#60. It does block claiming a certificate-backed ready profile or authoritative certified PR assessment. Until its candidate-certification trust model exists, lock-changing candidate PRs fail closed or report an explicitly unsupported outcome; they never receive a reusable signing secret or publish under a trusted default-branch lock tag.

Ayni CLI work remains independently planned in the
[Ayni CLI Project](https://github.com/orgs/ayni-oss/projects/1). A public Ayni
issue may be linked only when it needs a separately justified public capability;
it must not expose private Orchestrator material.
