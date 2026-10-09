# Repository-selected official builders

The topology below is implemented behavior. Disposable VM execution, remote execution destinations, and separate task-stage pods
are alternatives under [research #128](https://github.com/ayni-oss/orchestrator/issues/128), not additional guarantees of this deployment.

Each committed `.ayni.lock` selects an explicitly supported Ayni contract.
The compatibility catalog currently admits Ayni 0.15.0. The control plane resolves
`ghcr.io/ayni-oss/ayni-builder:0.15.0` to the requested Linux platform manifest and
freezes the digest before any validation, trust preparation or runtime build.
Repositories cannot select arbitrary builder images. Unsupported versions receive
upgrade guidance; owners update policy and lock with supported Ayni tooling.

Repository access and onboarding are distinct persisted states. Connecting discovers
access and a default branch; Analyze repository freezes a commit and inspects both
policy and lock from that commit. Missing or incompatible inputs stop preparation
with an actionable finding. Retry resumes the operation without authorizing image
replacement. Closed trust PRs remain visible and can be retried.

Matching official tooling validates policy and lock. Trust preparation receives
public certificate data only and changes only trust policy and its derived lock
values. An already trusted certificate needs no PR. Otherwise the owner reviews and
merges the trust update, after which the actual merged commit is analyzed again.
The trusted launcher and checksum-pinned mise are injected from the worker into
writable volumes; the worker carries no Ayni CLI or builder recipe.

Existing runtime images are independently verified. Only authenticated manifest
absence permits automatic building. Authentication failures, rate limits and
outages remain failures. An invalid existing image requires explicit Rebuild.
A successful build is certified, published, independently verified and cleaned up
before activation. Source, policy, lock fingerprint, builder digest, platform,
registry and certificate revision remain frozen for each generation. Stale results
cannot activate changed configuration; running tasks retain their admitted identities.

## Engines and security modes

Every attempt creates a Pod, disposable `emptyDir` Docker storage, internal Service,
NetworkPolicies, and an immutable Secret with newly generated mutual TLS identities.
The build and publication Jobs share that attempt's engine. Completion and failure
cleanup remove those resources. Engine containers mount only CA and server keys;
client Jobs mount only CA and client keys. The engine does not mount signing or
registry credentials, host storage, runtime sockets, or a service-account token.

The deployment administrator selects `runtime.engineMode`: `external-isolated`
(the default, with no Kubernetes engines), `local-development`, or
`dedicated-builder`. These infrastructure settings remain administrator-managed.
Repository lock versions select builders independently. Changing infrastructure
invalidates affected readiness and requires new verification. Removed global
fields must be removed from Helm values explicitly; use `--reset-values`.

Both Kubernetes modes use the same privileged DinD lifecycle, overlay2 storage
driver and resource limits. The builder nodes must support nested overlay mounts;
not every Kubernetes runtime is supported.
`local-development` runs on Docker Desktop with catalog-selected official builder
digests. Local success proves functionality only.

`dedicated-builder` requires Linux nodes of the selected architecture labelled
`ayni.io.node-restriction.kubernetes.io/builder=true`. Provision the label through
an administrator with NodeRestriction enabled and taint those nodes
`ayni.io/builder=true:NoSchedule`. Engines have a required node selector and only
that taint's toleration; unavailable builder capacity leaves engines Pending and
the attempt fails on startup timeout. There is no scheduling fallback.
The control plane and credential-bearing Jobs require that the protected label
is absent, even if the taint is accidentally removed. Configure trusted node
capacity and storage accordingly. Provision new builder nodes without trusted
workloads; drain reused nodes first, because scheduling affinity does not evict
already running Pods when labels change. These are scheduling controls, not proof
of isolation.

The recommended deployment is a separate builder Kubernetes cluster on disposable
VMs, with trusted orchestration and credential-bearing Jobs on separate trusted
nodes. The current adapter uses its configured Kubernetes client/namespace for all
attempt resources: deploy that adapter in the builder cluster; it does not route
engines into a second cluster from an application-cluster installation. A dedicated
node pool in the application cluster reduces exposure but retains cluster-credential
risk. Cluster administrators own admission policy, node credentials, VM disposal,
and the actual infrastructure boundary. Restrict builder node identities to their
own nodes and pods and prohibit access to trusted-node Secrets.

Attempt-specific NetworkPolicies admit only that attempt's clients on engine port
2376 and deny inbound connections to the client Jobs. DNS and HTTP/HTTPS egress
remain available for dependencies and registries, including private endpoints.
Infrastructure administrators must enforce firewall/CNI restrictions preventing
production-service, metadata, and control-plane access, including nested-container
traffic and both IP families. The generated policies do not establish those
protections; there is no activation checkbox for network isolation.

Privileged engines can compromise their node. An engine and credential-bearing Job
sharing a node cannot maintain credential isolation after node compromise. Separate
node placement prevents that co-location in dedicated mode, but a compromised node
still carries cluster credentials and may attack cluster infrastructure. mTLS and
Secret mount separation do not remove that risk. Local mode explicitly shares the
Docker Desktop node and has this limitation.

Engine loss discards its candidate, cleans up the old attempt and retries with a
fresh engine identity and empty storage, within the retry budget. Cleanup waits for Job dependents and engine Pod
deletion before readiness; restart resumes durable progress or cleanup.

## Deployment and recovery

`runtime.managedRuntime.build` contains engine infrastructure and service account
references. The compatibility catalog owns builder identity independently from
infrastructure and the orchestrator's CI tool pin in `scripts/ci/ayni-tool.json`.
`runtime.engineMode` retains explicit external, local-development and dedicated
builder boundaries. Source, signing, push and pull credentials stay separate.

Upgrade the server, generated clients, BFF and UI together. Back up private state,
stop obsolete operations, and supply cleaned Helm values with `--reset-values`.
Removed deployment fields are rejected rather than imported into active settings.
Migration preserves repository connections, certificates, registry selections,
runtime history and retained tasks while retiring obsolete global operations.
Retries and restarts resume durable onboarding and cleanup. See [operations](operations.md).
