# Assurance boundaries

Product direction agreed 2026-10-09. Independent assessment and execution-topology changes remain research in
[issue #128](https://github.com/ayni-oss/orchestrator/issues/128).

## Claims and evidence

| Claim | Evidence and limit |
| --- | --- |
| Official builder identity | Release provenance for the expected builder digest, publisher, platform, and version. Upstream startup verification is requested in [Ayni #75](https://github.com/ayni-oss/ayni/issues/75), not implemented here. |
| Verified repository environment | Trusted Ayni certificate, matching lock and protected content, compatible platform, and admitted image digest. This does not certify editable source quality. |
| Quality result | Full Ayni evidence bound to source, policy, environment, and final commit. Results measure the configured tools and tests, not universal correctness. |
| Independent assessment | A fresh assessment under authority the coding process cannot modify. This is a planned mode, not a property of the current worker check. |
| Merge approval | Repository policy and authorized reviewers decide whether a change may merge. A certificate or delivered draft does not grant approval. |

Signature validity alone is insufficient. Consumers must match the expected artifact and inputs, trust the signer,
and apply the organization's current acceptance policy.
Environment compatibility and authenticated evidence do not establish regulatory compliance.

## Current trust boundary

The trusted worker coordinator runs mandatory checks and retains final evidence outside the coding sandbox.
Repository commands, including tests, run under the documented Landlock restrictions.
Task code has no environment-signing seed or registry-push credentials.
The [worker contract](runtime-runner.md) describes recovery and evidence binding.

Repository-controlled tests and build scripts can still affect what is measured.
The certificate covers the environment's protected content; it does not make arbitrary test assertions sufficient.
A later PR revision cannot inherit evidence for an earlier commit.

## Optional independent CI assessment

Customers may require a second assessment with separate authority and an auditable result.
The same Ayni tool can serve that role; independence comes from control of the assessment.
The design must establish:

- A fresh checkout of the exact candidate revision and an approved runtime.
- Assessment policy and workflow definitions the coding task cannot weaken.
- Separate execution identity and credentials, with evidence retained outside candidate write access.
- Explicit source, policy, environment, assessor, and result identities; changed heads make prior results stale.
- Required human approval where customer policy calls for it, distinct from automated checking.

No independent-assessment service is implemented by this documentation change.
Existing repository CI remains usable, but its mere presence does not establish independence.

## Execution isolation research

GitLab Dedicated uses a disposable VM per job, including privileged Docker-in-Docker workloads.
Its documentation describes network separation and destruction after each job.
GitHub warns that self-hosted runners can be persistently compromised by untrusted workflows.
These are architectural references, not blanket approval of a customer's deployment.

Research compares disposable VMs, dedicated build clusters, dedicated nodes, and separate task-stage pods.
Node separation reduces co-location risk but does not remove cluster-credential or network risk.
A writable shared volume can carry hostile Git configuration, hooks, symlinks, and modified evidence between stages.
Stage separation therefore requires validated immutable handoffs and narrowly scoped authority.

The existing privileged build-engine limitations remain in [repository builders](../reference/repository-builders.md).
Operators still own infrastructure isolation, access control, egress restrictions, patching, and disposal.
No research proposal authorizes a fallback onto trusted nodes or relaxes current worker sandboxing.

## References

- [GitLab Dedicated hosted runners](https://docs.gitlab.com/administration/dedicated/hosted_runners/): disposable execution and documented network boundaries.
- [GitHub secure use](https://docs.github.com/en/actions/reference/security/secure-use): risks of self-hosted execution.
- [SLSA build requirements](https://slsa.dev/spec/v1.2/build-requirements): provenance authenticity, isolation, and signing-authority separation.
