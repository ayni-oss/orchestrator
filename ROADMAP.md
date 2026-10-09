# Roadmap

Current requirements and delivery status live in the private
[Orchestrator Project](https://github.com/orgs/ayni-oss/projects/2) and linked issues.
The [Polish Ayni development flow milestone](https://github.com/ayni-oss/orchestrator/milestone/8)
collects the current documentation, execution-isolation research, and environment-change work.

| Work | Source of truth |
| --- | --- |
| Product model and assurance documentation | [#127](https://github.com/ayni-oss/orchestrator/issues/127) |
| Disposable execution, stage separation, and independent assessment research | [#128](https://github.com/ayni-oss/orchestrator/issues/128) |
| Reviewed environment changes, candidate validation, and shared runtime reconciliation | [#129](https://github.com/ayni-oss/orchestrator/issues/129) |

The [product flow](docs/ayni-product-flow.md) explains ownership and user behavior.
The [architecture](docs/architecture.md) describes implemented execution.
Research issues do not authorize changes to the current worker or trust boundaries.

Completed delivery remains in [closed milestones](https://github.com/ayni-oss/orchestrator/milestones?state=closed),
merged pull requests, and the [historical decision index](docs/decisions/README.md).
Historical assessment pipelines and environment workflow installation are not the active plan.

Ayni remains independently useful and publicly planned in its own repository.
[Public builder-attestation feature #75](https://github.com/ayni-oss/ayni/issues/75)
is an upstream dependency to evaluate, not a capability already available to Orchestrator.
