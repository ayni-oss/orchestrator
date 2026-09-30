# Protocol release state

Buf baseline generation: `clean-v1` (see `proto/BASELINE`).

| Package | State | Compatibility boundary |
| --- | --- | --- |
| `ayni.orchestrator.v1` | additive-only from the clean-v1 baseline | Field numbers, names, RPC signatures, streaming shapes, and established semantics follow the rules in `README.md`. Incompatible changes require a new package. |

The clean-v1 generation intentionally replaces the unused private-pilot
baseline. Protobuf CI skips comparison only while the base branch has no
`proto/BASELINE` marker; this is the one-time reset for the clean-v1 replacement.
Once merged, later changes retain the `clean-v1` marker and Buf compares them
with the established clean-v1 base contract. CI rejects future marker changes;
another reset would require a separate explicit contract and CI decision.

Custom-client bundles identify this package plus the immutable private source
revision from which they were built. Client authors pin that revision, verify
bundle checksums, and regenerate before upgrading; additive-only does not mean a
client may silently depend on a field that its deployed server does not yet
provide.

The package state is independent of server deployment and generated SDK
publication. Changing this table or building a descriptor bundle does not
publish an endpoint, package, image, or repository.
