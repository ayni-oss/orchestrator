# Ayni 0.14 integration contract

This document records the released Ayni contract consumed by the Ayni Managed
profile. It supersedes the exploratory assumptions recorded for #53; it is not
a replacement implementation of Ayni's lock, certificate, or protected-content
verification.

## Released baseline

| Item | Pinned value |
| --- | --- |
| Ayni release | `ayni-v0.14.0` |
| Linux x86_64 archive | `ayni-ayni-v0.14.0-x86_64-unknown-linux-gnu.tar.gz` |
| Archive SHA-256 | `5172ab4e530fda3eb1a18dac90595adb6b342c9b9773fa6e98ba4f5d7dd2058e` |
| Supported managed platforms | `linux/amd64`, `linux/arm64` |
| Repository lock | Ayni-generated committed `.ayni.lock`, schema `0.9.0` and recipe `2` |
| Discovery tag | `lock-<full-64-hex-lock-fingerprint>` |

The complete lock fingerprint covers the contract and recipe digests, Ayni and
Mise versions, provisioning base and variant, platforms, runtimes/components,
signal tools, dependency inputs, capabilities, and resource limits. It is not
a repository revision or an OCI digest. Source-only edits reuse an unchanged
lock artifact; changing a covered input requires a new lock and artifact.

## Launcher and runtime boundary

`ayni env build` consumes the committed current lock and produces a reusable
OCI image. It does not publish or launch the image. Platform tooling owns
registry publication, digest resolution, pull credentials, network policy,
resource limits, user identity, checkout attachment, and the launcher.

The platform pulls the admitted image by `repository@sha256:<digest>`, attaches
the writable checkout before startup, preserves the image entrypoint, and
invokes plain `ayni check` in that checkout. The entrypoint may materialize
locked dependency seeds and caches, but must not replace sources, refresh native
locks, or silently delete existing dependency directories. No Docker engine,
host socket, Kubernetes token, controller state, package-write credential, or
certificate-signing seed belongs in coding or assessment containers.

When `/etc/ayni/runtime.json` is present, Ayni verifies its runtime metadata,
the attached checkout lock, platform, protected content, and optional
certificate before activating locked tooling. Ayni records its verified runtime
context in `signals.json`; that context proves Ayni compatibility, not launcher
isolation, trusted build authority, or authoritative PR assessment.

## Certification and admission

An unsigned `ayni env build` establishes only local lock/protected-content
consistency. A trusted default-branch publication supplies both
`AYNI_ENV_CERTIFICATE_SIGNING_KEY` and `AYNI_ENV_CERTIFICATE_KEY_ID` from
trusted CI. Ayni rejects absent pairs, malformed values, or a signing key whose
public key does not match the lock-pinned trusted key. The controller still
independently verifies package ownership, full-fingerprint discovery tag,
resolved digest, platform, expected Ayni metadata, and the trusted workflow
identity before admitting a runtime artifact.

Candidate code never receives the reusable signing seed or package-write
authority. A candidate lock change is unsupported until a separately reviewed,
isolated candidate trust policy exists; it must fail closed rather than use
unsigned mode as a bypass.

## Assessment evidence

The workflow—not Ayni or candidate code—owns the assessment manifest. It binds
independently observed repository, base SHA, exact PR head SHA, profile revision,
OCI digest, workflow run and attempt, source-integrity result, exit status,
completion state, and hashes of retained evidence. It retains the original
`.ayni/last/signals.json`; `execution.json` is retired and must neither be
required nor synthesized.

Candidate-controlled bytes, including `signals.json`, are inputs to a trusted
collector. Hashing those bytes in the candidate's process is insufficient. The
collector runs after candidate code with separate write authority, bounds and
validates artifact paths and sizes, computes hashes itself, and records the
workflow identity before the controller imports an assessment. The controller
rejects missing, malformed, oversized, mismatched, stale-head, or conflicting
evidence. A GitHub check, comment, agent result, successful unsigned build, or
`managed` execution-mode value is a projection or compatibility observation—not
an authoritative assessment.

## Reproducible validation harness

The repository PR workflow verifies the pinned release archive, uses Ayni to
build the lock-derived image, then uses Docker to attach the exact checked-out
source at `/workspace` and run plain `ayni check`. It intentionally has only
`contents: read` and produces ordinary PR feedback; it neither publishes an
image nor claims certified or authoritative assessment status.

The managed-profile delivery workflows in #58/#59/#62/#63 must additionally
record bounded timing and resource observations for cold build, warm build,
registry reuse, clean-node pull/start/check, and all rejection cases. They run
only with an explicitly authorized sandbox package and the supported Actions
topology. Docker Desktop validation uses the explicit `docker-desktop` context;
host-only checks and local image caches are not deployment proof.
