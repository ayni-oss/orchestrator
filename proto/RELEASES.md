# Protocol release state

The active private baseline is `repository-builders-v4` (see `BASELINE`).

This intentional cutover removes the deployment-wide Ayni settings, installation,
release picker and service-test RPCs and messages. Repository connections expose
persisted onboarding status, reason and inspected revision. AnalyzeRepository
requests analysis/retry independently from provider access and runtime replacement.
Upgrade the server and regenerated clients together.

The authorized comparison reset applies only from `managed-images-v3` to
`repository-builders-v4`. Subsequent changes compare against the established
baseline with Buf breaking checks. Another reset requires an explicitly approved
breaking contract change. Private bundles identify source revisions and checksums;
creating descriptors does not publish endpoints or SDKs.

The prior baseline removed repository environment workflow installation resources.
Earlier worker-owned task contracts, frozen execution limits and publication retry
remain. Persisted history and retained task work survive migration; see
[operations](../docs/operations.md).
