# Managed runtime execution

Runtime execution is opt-in. The reusable application chart and the local
umbrella expose `runtime` values (under `ayni-orchestrator.runtime` in the
umbrella). The chart creates `runtime-coding` and `runtime-builder`
ServiceAccounts by default. They have no API token mounts or RBAC grants.
Set `runtime.serviceAccounts.create=false` and supply each profile's
`service_account` (and build `service_account`) to use existing accounts.
`runtime.serviceAccounts.imagePullSecrets` contains existing Secret names;
Helm never creates registry passwords or signing keys.

Example application-chart values (replace the placeholder worker digest):

```yaml
runtime:
  enabled: true
  workerImage: registry.internal/orchestrator/worker@sha256:<worker-manifest-digest>
  intervalSeconds: 300
  requestTimeoutSeconds: 30
  concurrency: 4
  outputClaim: runtime-outputs
  modelSecret: openai-api
  serviceAccounts:
    create: true
    imagePullSecrets:
      - name: runtime-pull
  managedRuntime:
    platform: linux/amd64
    service_account: runtime-coding
    pull_secret: runtime-pull
```

Connected repositories enrolled through managed onboarding automatically resolve
an image in their own `ghcr.io/<owner>/<repository>-ayni` namespace using the
selected lock fingerprint. Onboarding is the repository enrollment boundary;
operators supply deployment-wide execution policy, not a second repository list.
Explicit `runtime.profiles` entries take precedence and continue to support other
registry coordinates or dedicated build sources. The
controller reads the exact default-branch SHA and bounded lock through the
repository's GitHub App installation, creates an immutable source-policy
revision, and persists verification intent before creating a Job. A digest-pinned
trusted worker init container checks out that SHA with a contents-read token.
Only this init receives the token; it never persists it in Git configuration.

Kubernetes pulls the selected runtime using its own authentication. The image's
entrypoint remains intact. Before admission, the mounted worker rejects unsigned
runtime metadata, invokes the image's trusted Ayni executable at the integration's pinned release, and binds
its fresh signals to the exact certificate, lock file, full fingerprint,
platform, and version. Ayni—not another controller implementation—performs the
signature and protected-content verification. Quality findings do not invalidate
an otherwise verified runtime. The repository-pinned public key verifies the
certificate; the signing seed belongs only to the producer.

The admitted digest comes from Kubernetes' repository-qualified `imageID`.
An unqualified image/config ID is deliberately rejected rather than mistaken
for a pullable manifest digest. The configured registry, Kubernetes runtime,
and trusted Ayni release are part of this happy-path trust model. This is not
an offline verifier for arbitrary hostile executables in user-supplied images.
Custom Runtime remains a separate profile variant and delivery path.

Bindings are immutable per source policy, lock fingerprint, and platform. Tag
drift degrades the profile without rewriting an existing binding. Source-only
changes re-verify the existing environment without a build. Lock/source-policy
changes create an updating revision; running tasks retain their frozen digest.
Task admission currently requires the exact SHA whose compatibility was observed.
Profile `reconciliation_metadata` exposes source, SHA, fingerprint, reference,
digest, build Job, last success, next retry (Unix seconds), and safe reason codes.

## Dedicated builds

Add an explicit build configuration to the profile to build an unpublished
environment. There is no implicit fallback from a registry-source failure.

```yaml
build:
  image: registry.internal/orchestrator/builder@sha256:<builder-manifest-digest>
  engine_host: tcp://isolated-build-engine.internal:2376
  engine_secret: build-engine-tls
  registry_secret: environment-push
  signing_secret: environment-producer
```

Released `ayni env build` uses an OCI engine. The dedicated, unprivileged Job
uses an **operator-provisioned, isolated TLS Docker engine**; Helm does not
provision that engine. The build Job has no host socket, privileged container,
controller volume, model key, or Kubernetes token. The engine must be dedicated
to trusted build work, authenticate its clients, and enforce its own resource
and storage policy. A generic Docker endpoint is not a least-privilege build API.
Do not point this at a host/shared engine containing unrelated workloads.

`deploy/images/runtime-builder/Dockerfile` combines the pinned released Ayni
executor with an explicitly selected Docker CLI image. Build and distribute
this application component through the operator's private image process, then
configure its manifest digest. The builder requires a compatible Docker API
and BuildKit-capable engine. This transport still needs live environment proof;
it is not a claim of daemonless Ayni support.

The engine Secret provides `ca.pem`, `cert.pem`, and `key.pem`. The registry
Secret supplies `.dockerconfigjson`; the signing Secret supplies `signing-key`
and `key-id` for Ayni's supported certificate environment variables. For the
local Docker Desktop environment, `dev/ctl secrets` creates this Secret as
`environment-producer` from the optional `AYNI_ENV_CERTIFICATE_SIGNING_KEY` and
`AYNI_ENV_CERTIFICATE_KEY_ID` entries in the mode-600 `dev/.env`; set
`build.signing_secret` to that name. Other installations provision the referenced
Secret through their secret manager. Only the build Job references those
credentials. It runs `ayni env build --tag` and then pushes that tag to the
configured private registry. The controller neither executes image-building
tooling nor reads registry/signing Secret contents.
Kubernetes Secret/ServiceAccount separation is deployment wiring, not a defence
against a compromised controller with permission to create arbitrary Pods;
stronger tenancy requires admission policy and separate namespaces.

Build intent is saved before launch. Unknown/failed completion is observed
through registry verification before any further decision; the controller does
not automatically repeat an uncertain build. Repair build configuration (a new
source-policy revision) to authorize a replacement. Jobs have deadlines and no
automatic Kubernetes retries. Successful verification is required even after
a successful push. No GitHub package publication or PAT is required.

## Coding, output, and publication

Each attempt freezes source, profile revision (including the full lock
fingerprint), OCI manifest digest, platform, model, constraints, and deadline
before creating its Job. The shared runtime renderer preserves the entrypoint
and non-root/read-only-root isolation. Required init support installs the worker,
checks out the frozen source, and prepares the attempt's output directory.
The coding container sees only its own output subdirectory, writable checkout,
scratch, read-only request/launcher, and model key. It never receives GitHub,
registry-push, build-engine, signing, or controller credentials.

The model receives repository-instruction, constraint, and advisory-check
instructions. Its shell tools clear inherited credentials, bound retained output,
and time out after 180 seconds. The agent has at most 24 turns. The launcher runs
a final advisory check independently of the model's completion prose and reports
pass, failure, or unavailable/incompatible status without fabricating success.
Policy/lock edits can make the frozen runtime incompatible; no rebuild occurs.
Protected paths are rejected. Without test-edit permission, dedicated test files
and changes to Rust files containing inline tests are conservatively rejected.

Generated `.ayni` and `.orchestrator` output is excluded even if the model staged
it. The launcher freezes a signed-off candidate commit and a bounded Git bundle.
This is not a cryptographic producer signature or assessment evidence. Usage
records contain counters, never prompts. Requested input is surfaced on the
attempt; a new task can supply clarified instructions. A retry retains the
original task snapshot; a new task selects the current ready profile.

The controller collects results only after Job completion, checks the retained
bundle digest, and records the candidate separately from the attempt. Cancellation
and deadlines reject late results. Missing running Jobs fail instead of silently
restarting model work. Output claims are retained; their cleanup remains the
retention policy's responsibility. The local ReadWriteOnce claim assumes the
single-node Docker Desktop topology; multi-node operators need compatible shared
storage or a future object-store output transport.

An explicit `PublishCandidate` request resumes through the shared durable action.
The controller imports the bundle into a fresh bare Git object database, verifies
the frozen commit and ancestry, and pushes a deterministic branch without
overwriting a conflicting branch. It does not check out or execute candidate
code. Provider lookup precedes PR creation so an uncertain response can be
recovered. The PR body carries the advisory disclosure; no pre-publication
managed validation Job or assessment is synthesized. Exact-head authoritative
assessment uses the separate [managed PR workflow](managed-pr-assessments.md).

## Validation boundary

Run workspace tests, strict Clippy, protocol generation checks, frontend checks,
and Helm validation locally. Registry-source validation needs a published signed
image. Build-source validation does not need an existing environment image: it
needs the configured builder image, isolated engine, destination registry, and
scoped registry/signing Secrets to produce one. Those sandbox runs are deferred
until that infrastructure is supplied;
unit tests, manifest tests, and application image builds are not substitutes
for multi-node pull/build/coding/PR proof.
