# Protocol release state

The active private baseline is `managed-images-v3` (see `BASELINE`).

This intentionally breaking cutover removes repository environment workflow
installation RPCs and resources. Removed field numbers and names are reserved.
Runtime profile reads add findings, preparation phase and rebuild availability;
inspection adds findings. Deploy the server and regenerated clients together.
Schema-six and schema-seven databases migrate transactionally to schema eight,
preserving connections, repositories, task snapshots and retained task work while
retiring workflow intents. See [operations](../docs/operations.md).

The earlier `worker-owned-v2` baseline introduced worker-owned task results,
frozen execution limits and task publication retry. Those task contracts remain;
this cutover adds no environment publish-only action or unpublished image API.

The authorized comparison reset applies only from `worker-owned-v2` to
`managed-images-v3`. Subsequent changes compare against the established baseline
with Buf breaking checks. Another reset requires an explicitly approved breaking
contract change. Private bundles identify source revisions and checksums; creating
bundles or descriptors does not publish endpoints or SDKs.
