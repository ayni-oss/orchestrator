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

Ayni CLI work remains independently planned in the
[Ayni CLI Project](https://github.com/orgs/ayni-oss/projects/1). A public Ayni
issue may be linked only when it needs a separately justified public capability;
it must not expose private Orchestrator material.
