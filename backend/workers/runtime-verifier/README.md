# Trusted runtime content verifier

The controller mounts this verifier separately inside the exact immutable candidate
image, with its root filesystem and the frozen source directory read-only. It runs
as an unprivileged user with no signing, source-provider, registry or cluster
credentials and no network. The controller owns time, memory and CPU limits.

```sh
/ayni-runtime-verifier --source /source \
  --expected-platform linux/arm64 \
  --expected-cli-sha256 sha256:<trusted-official-executable-digest>
```

The executable digest must come from independently trusted official builder
evidence. Computing an expected digest from the candidate itself provides no
independent executable identity. The platform defaults to the compiled host
architecture; only Linux amd64 and arm64 are admitted.

The verifier reads and hashes the candidate's Ayni executable without executing it.
Pinned Ayni 0.16.2 APIs validate committed policy and lock, trust, certificate
signature and claims, protected manifest and actual protected content. Every
response is bounded JSON. Exit 0 with `state: content_verified` establishes only
certification and protected-content integrity. Environment discovery, registry
readback, managed compatibility, cleanup and runtime activation remain separate
controller gates. Exit 2 with `state: invalid` returns a safe reason enum.

Source limits are 256 regular files, 512 KiB per file, 8 MiB total, 512 filesystem
entries and 64 directory levels. Source symlinks are rejected. Metadata is bounded
using Ayni's public limits; protected regular files are limited to 256 MiB each,
8 GiB total and Ayni's public manifest entry count, with at most 128 directory
levels. Protected-tree symlinks retain Ayni's official verification semantics.

The packaging Dockerfile builds with musl and fails if ELF inspection finds a
program interpreter or shared-library dependency. Successful image construction
and an actual candidate-root acceptance run are required deployment evidence;
host rejection tests alone do not qualify managed content verification.
