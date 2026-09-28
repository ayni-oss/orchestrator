# Protocol release state

| Package | State | Compatibility boundary |
| --- | --- | --- |
| `ayni.orchestrator.v1` | additive-only | Field numbers, names, RPC signatures, streaming shapes, and established semantics follow the rules in `README.md`. Incompatible changes require a new package. |

Custom-client bundles identify this package plus the immutable private source
revision from which they were built. Client authors pin that revision, verify
bundle checksums, and regenerate before upgrading; additive-only does not mean a
client may silently depend on a field that its deployed server does not yet
provide.

The package state is independent of server deployment and generated SDK
publication. Changing this table or building a descriptor bundle does not
publish an endpoint, package, image, or repository.
