# Ayni Managed OCI workflow research

- Issue: [#53](https://github.com/ayni-oss/orchestrator/issues/53)
- Date: 2026-09-29
- Tested Ayni release: `0.13.0`
- Status: completed; the issue's closure records the direct-OCI boundary

This is an integration record, not an Ayni implementation contract.
Orchestrator consumes the published environment as a verified OCI image by
digest. It does **not** ask Ayni to import, restore, or launch that image as an
Ayni managed environment. Inside the coding container, `ayni check --host` is
advisory agent feedback, not managed assessment evidence.

## Public inputs and metadata

Ayni `0.13.0` provides `env lock`, `env build`, `env doctor`, and `env storage`.
`env build` creates a local OCI image and `.ayni/environment/build.json`; the
record is useful build evidence but is not required by the direct OCI runner.
`env doctor` validates that record and image rather than trusting a mutable tag.

The current implementation emits and validates these local image labels. The
Ayni Managed publisher/consumer verifies them against its committed lock rather
than recreating Ayni's interpretation:

| Label | Expected value |
| --- | --- |
| `dev.ayni.environment.owner` | `ayni` |
| `dev.ayni.environment.schema` | `0.7.0` |
| `dev.ayni.environment.lock-fingerprint` | Full `sha256:<64 hex>` committed-lock fingerprint |
| `dev.ayni.environment.base-digest` | Lock provisioning-base digest |
| `dev.ayni.environment.ayni-version` | Installed Ayni version |
| `dev.ayni.environment.mise-version` | Lock provisioning Mise version |
| `dev.ayni.environment.platform` | `linux/amd64` or `linux/arm64` as applicable |
| `dev.ayni.environment.preparation-digest` | Full prepared-input digest |

The lock fingerprint is canonical JSON over its complete lock contract: schema
and recipe, contract path/digest, Ayni and Mise versions, provisioning base,
platforms, target runtime/tool inputs, adapter-owned dependency inputs,
capabilities, and resource ceilings. It is not a source revision. Ordinary
source edits are not lock inputs; edits to a declared contract,
runtime/tool/dependency input, capability, resource, Ayni version, or Mise
version require `env lock` and can change the fingerprint. Ayni rejects a
mismatched serialized fingerprint.

Ayni's local image tag deliberately includes truncated lock/preparation segments
plus platform/executor information. It is a local engine convenience, not the
registry naming contract. Ayni Managed tags the verified image externally as:

```text
ghcr.io/<owner>/<repository>/ayni-environment:lock-<full-64-hex-lock-fingerprint>
```

The controller resolves that discovery tag once, verifies repository ownership,
labels, and platform, then freezes the OCI manifest digest in each task. It
never uses the tag as task identity.

## Executed evidence

The following was run from a fresh local clone with the published `ayni 0.13.0`
macOS arm64 binary. The release archive passed its published SHA-256 checksum.
Docker Desktop `29.8.1`, Buildx `0.37.1`, and `linux/arm64` were available.

```sh
ayni env build --repo-root .
ayni env doctor --repo-root .
ayni env storage --repo-root . --output json
```

The cold build completed in **96 seconds**, produced a 2,467,485,058-byte
cumulative local image, and recorded:

```text
lock fingerprint: sha256:5e07813072d0f5582be9af22fe25d45d8f995b84cf5557d958d7a62bc8c3d391
image ID:         sha256:05e00df2ce3cf95002526ce387860d6388cccd5f800fead11384af1491195b34
platform:         linux/arm64
image schema:     0.7.0
preparation:      sha256:6dda853e2704eb55adeae73dbbb308eb359aac4d78a83054f9a208c4072ec0ff
```

The first-build executor was
`ghcr.io/ayni-oss/ayni-env:0.13.0-debian@sha256:e21ccf9e66e7490953d2a30ba49ce8894025fd0e59556bc65a3c17c783ae5fe9`.
The Ayni release archive is checksum-pinned by the consuming workflow; the
GitHub-hosted MVP runner is `ubuntu-24.04`/`linux/amd64` and needs Docker Buildx.
The observed local result is capacity evidence only, not an amd64 timing claim.

A clean clone with the local image but no Ayni build record failed `env doctor`
with `execution build record is missing`. That is expected and is not an
Orchestrator blocker: direct OCI execution uses the verified image digest,
rather than asking Ayni for a managed check from that image.

The tested image entrypoint is `ayni`; host feedback is invoked as:

```sh
docker run --rm \
  --volume "$PWD:/workspace" --workdir /workspace \
  <verified-image@sha256:...> check --host --config .ayni.toml
```

The workspace must be writable by the image execution user and its cache
ownership must be compatible. A host-UID override on the tested image failed
because `/home/ayni/.cache/cargo` was not writable; moving cache variables broke
the image's Mise shim. The shared runner (#56) must prepare a compatible
workspace rather than assume arbitrary host-UID compatibility. This command is
feedback only and does not make a managed assessment.

## Delivery sequence

The following sequence is the executable boundary for #59, #61, and #62. It
runs only in the enrolled repository package; pull-request code never receives
package write.

1. On trusted default branch at exact `BASE_SHA`, install checksum-pinned Ayni
   `0.13.0`, run `env build` and `env doctor`, capture `build.json`, and inspect
   local image labels.
2. Retag the verified local image using the full lock fingerprint and push it to
   the enrolled repository's GHCR package. Resolve the pushed tag to an OCI
   manifest digest and record its platform/labels.
3. On a clean `ubuntu-24.04` runner or Kubernetes node, pull the resolved digest
   and run the coding container directly from `image@sha256:...`. No `env build`
   runs in that container.
4. For unchanged-lock PRs, managed PR automation pulls the trusted
   default-branch digest and runs managed `ayni check` against the exact head.
   For a changed lock, it uses an isolated candidate environment with no package
   write token; it cannot publish or replace the trusted tag.
5. Existing valid lock tags resolve to their recorded digest; moved/missing tags,
   wrong owner/label/platform, or a different digest fail closed. A source-only
   default-branch revision verifies/reuses the artifact without build/push.

Start with a 15-minute uncached-build timeout and 10 minutes for pull plus
managed check; #59/#61/#62 must replace those starting points with recorded
`linux/amd64` cold, warm, registry-reuse, and second-node timings. No Ayni
source change or private reimplementation is required by this workflow.
