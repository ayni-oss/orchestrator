# Protocol release state

| Package | State | Compatibility boundary |
| --- | --- | --- |
| `ayni.orchestrator.v1` | additive-only | Field numbers, names, RPC signatures, streaming shapes, and established semantics follow the rules in `README.md`. Incompatible changes require a new package. |

The package state is independent of server deployment and generated SDK
publication. Changing this table does not publish an endpoint, package, image,
or repository.
