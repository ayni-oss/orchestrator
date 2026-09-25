# M4 engineering-gate record

> Historical delivery record. Milestone names, resource identities, and commands
> below describe the dated revision; use [local development](local-development.md)
> for current operation.

Status: all applicable engineering gates passed on the selected Docker Desktop pilot topology on 2026-09-25. This establishes implementation readiness for M5, not product value.

## Candidate and topology

- Runtime source: `e17e932fd1c21c43fec1912b84197c4c95c1fc56`
- Control plane: `local/ayni-orchestrator-control-plane:git-e17e932fd1c2`
  - OCI index: `sha256:85cf0a9b791fe1e1e1b91a9c6cb1bc55322b2f538c9a916834632753b6edf774`
  - running image: `sha256:bee47efb009b298efcbf183a737cdfc21546809dd0fb4d1118535a4838709f93`
- Worker: `local/ayni-orchestrator-worker:git-e17e932fd1c2`
  - OCI index: `sha256:67f883d92d967b76a195c1652857ef5d6f0624f4610d6efe5933b2bf62285cb6`
- Docker Engine client/server: `29.8.0`
- Kubernetes client/server: `v1.36.1`; Kustomize: `v5.8.1`
- Context/node/namespace: `docker-desktop`, `desktop-control-plane`, `ayni-orchestrator-dev`

The final source passed formatting, Clippy with warnings denied, all-feature workspace tests (33 passing), compile checks, shell syntax checks, M4 Kustomize rendering, client dry-run, and `scripts/local/check.sh m4`. A fresh isolated Job using the final worker revalidated candidate `46427c48…` from the retained prepared environment with `success`, three artifacts, preparation reuse, and manifest SHA-256 `8112f278132c21d1e05de5501f5c51a0887f0906de1de0290db179b1059b4826`; its probe directories were then removed. All four retained PVCs remained `Bound`. `pilot_ready: true` means publication features are configured; repository readiness is established separately by a `ready` preparation.

## Scenario results

Each row incorporates the corresponding required outcome from the
[engineering-gate table](mvp-validation.md#engineering-gates) as its expected
outcome; the middle column records the actual result and durable reference.

| Scenario | Actual outcome and evidence | Status |
| --- | --- | --- |
| Valid repository and request | The browser submitted task `01a0d8a1-ef61-7124-ba0a-9f50d1578640`, which produced candidate `6372f45a8da727db395893a541e65374db6dbb01`, manifest `sha256:487da11b516e05576ba71f86b1323533b9fbd415a158b08c6ecb3addfc953a71`, three digest-bound artifacts, and exactly one draft [`ayni-oss/sandbox#3`](https://github.com/ayni-oss/sandbox/pull/3). The rendered evidence action displayed those exact results. | Pass |
| Missing configuration, stale lock, or unsupported shape | M1 recorded stale-lock rejection. Binding `sandbox-revoked-access-probe` reports `setup_required` and a GitHub 404. Failed preparations are not presented as repository `ready`. | Pass |
| Matching preparation reuse | Revision `589fb2a2fae67d87e00da1ddc4c9a576a207d2ff` cold-prepared in 106 seconds as `prep-c536767dd800a652ba3f8e7a8c23aed7bda3e4c11e1edeb73b94fda063359d02`. Repeated API and browser actions returned that identity and raised `reuse_count` to 3. | Pass |
| Failed replacement | Enrollment lock was temporarily changed to all zeroes. Replacement `prep-085ef6186f424e17d0efdcf9dd58b5cb110df6addbc769487b104c443709ce77` failed, and task submission returned HTTP 400 `repository revision is not ready`; the older preparation was not substituted. Restoring the lock warm-reused the original identity. | Pass |
| Intentional quality failure | Task `01a0d4d8-1dc8-7694-acb1-b6f38f43316c` retained outcome `quality_failure`, manifest `sha256:0df39be36a89c9a6a4da58b3989ad15e47d171d18c58c95bd21d09fe2623b9c5`, and no PR. | Pass |
| Missing or altered final artifacts | For task `01a0d83e…`, `signals.json` was altered and `execution.json` was separately removed. Each evidence request returned HTTP 404 with the expected diagnostic. Exact backups were restored and the original manifest verified. Unit tests also cover execution-to-signals mismatch. | Pass |
| Candidate changes after validation | After task `01a0d8ab-ff83-72cb-9eef-580700e40430` completed its validation Job, but before validation success was recorded, its checkpoint bundle was replaced while the controller was stopped. On restart, the control plane failed it with `Validated candidate changed: candidate checkpoint bundle does not contain the validated candidate`; no validation success was recorded, no PR Job or PR was created, and the original bundle was restored. Unit tests cover identity and bundle-head changes. | Pass |
| Policy or test edits | Final-worker Job `m4-policy-test-edit-probe` changed `.ayni.toml` and an inline Rust test. It failed with `policy_rejected`, `policy_edited: true`, `tests_edited: true`, both paths, and no artifacts. | Pass |
| Duplicate scheduling and restart | During task `01a0d893-0e67-7702-8ea6-335445d19b54`, a control-plane restart retained coding Job UID `75ba89e4-1e66-44c6-bd89-8b6cdb1778c7`; no duplicate appeared. M2 evidence covers duplicate reports. | Pass |
| Worker disappears | The validation Job for task `01a0d885-26b5-713a-bdce-f506fe0ccc6e` was deleted. The task visibly failed, retained candidate `98adc86b655f8b25daeeae8a9186421d034eb984` and bundle digest `c44c12d7b5d6be116e46e6d8e73f71b90e281822ef68ee0ff81e94a1bc0fad26`, and created no PR. | Pass |
| Cancellation or limit | Restart probe task `01a0d893…` was cancelled and its coding Job deleted. Browser-created input follow-up `01a0d88b-20fc-701d-9a24-a710a1975779` was cancelled through its rendered action. Tests prove late completion cannot revive cancellation; Job deadlines remain one hour. | Pass |
| Uncertain PR response | M3 recovered a pending publication after replacement and produced only sandbox PR `#1`. M4 publication reconciliation returned existing PR `#2` despite changed checkpoint bytes. | Pass |
| Cleanup with active peer | With active M4 probe Job UID `6798d358-f49f-4eba-804f-d3bd434d7708`, normal cleanup removed only stateless control-plane resources. The same Job UID stayed active; all three credential Secrets and all four PVC UIDs survived. Reapplying the final manifests restored readiness, then the probe Job alone was deleted. | Pass |
| Repository access absent after readiness | After valid preparation, enrollment was temporarily changed to a nonexistent installation ID. Task `01a0d882-bf26-7770-9556-6a8faa789fbc` failed with an actionable GitHub 404; no coding Job, candidate, evidence, or PR was created. Enrollment was restored. | Pass |
| Dependency evolution | Final-worker Job `m4-dependency-probe-final` changed `Cargo.toml` and failed with `dependency_change_unsupported`, exit `-1`, and no artifacts. | Pass |

## Browser and requested-input walkthrough

Chrome executed the deployed page through the DevTools Protocol. It rendered readiness and history; submitted the preparation form and observed warm reuse; rendered waiting task `01a0d884-27b9-7511-a653-af10582edf6d`; clicked **Provide input**; observed a linked task; and clicked **Cancel** on follow-up `01a0d88b-20fc-701d-9a24-a710a1975779`. It then submitted bounded task `01a0d8a1-ef61-7124-ba0a-9f50d1578640`, followed it through coding, validation, and publication, clicked **View**, and displayed its success evidence and draft PR `#3`. The maintainer also opened the UI using the documented loopback port-forward.

The browser surface is unauthenticated inside the cluster and is operationally restricted to loopback port-forwarding for this trusted pilot. The service itself is an in-cluster `ClusterIP`.

## Installation and update

At runtime source `e17e932`, the separate `ayni-orchestrator-clean-check` namespace exposed empty task state, reported `pilot_ready: true`, and bound all four claims before it was deleted. Namespace and local-port overrides make that check reproducible. `pilot_ready: true` established publisher/control-plane configuration, not enrollment.

The M3-to-M4 update retained tasks `01a0d75c-5766-7509-b04d-ae856bdb56f0` and `01a0d4d8-1dc8-7694-acb1-b6f38f43316c` and PVC UIDs:

- control state: `3ffebb3b-1925-468e-b830-d73f8338ba1a`
- checkpoints: `0f046bbc-1d7e-4490-8430-c45e28d223db`
- managed evidence: `9b598de1-c274-41bf-9065-3938483072a9`
- prepared environments: `f4b0ff2f-9601-4a6d-a2ae-deb1e99adf06`

The rollouts from runtime `132998e` through `b42e86d` to signed runtime `e17e932` again retained those claims and records.

## Limitations

- This is a single trusted-team Docker Desktop pilot without end-user authentication or multi-tenant isolation.
- Access disappearing after token mint can surface as a generic coding failure; publication-time provider outages use deterministic reconciliation.
- There is no partial-work download API. The worker-loss exercise retained the completed coding checkpoint before validation interruption.
- Deployment requires at least 12 GiB free to avoid partial Docker storage failures.
- The delivery PR is created only when this local branch is explicitly published; it is not a generated sandbox output PR.
