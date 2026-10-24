-- Reclaim only source archives whose continued use has been durably superseded.
-- Commands, patches, quality evidence and approval/recovery receipts remain retained.
ALTER TABLE workflow_task_runs
    ADD COLUMN source_reclaim_after timestamptz,
    ADD COLUMN source_retired_at timestamptz,
    ADD COLUMN source_retired_bytes bigint NOT NULL DEFAULT 0 CHECK(source_retired_bytes >= 0);
ALTER TABLE workflow_task_runs DROP CONSTRAINT workflow_task_runs_source_pack_check;
ALTER TABLE workflow_task_runs ADD CONSTRAINT workflow_task_runs_source_pack_check CHECK (
    (source_retired_at IS NULL AND octet_length(source_pack) BETWEEN 1 AND 8388608 AND source_retired_bytes=0)
    OR (source_retired_at IS NOT NULL AND octet_length(source_pack)=0 AND source_retired_bytes>0)
);
CREATE INDEX workflow_task_runs_source_retention_idx ON workflow_task_runs(source_reclaim_after,attempt_id)
    WHERE source_retired_at IS NULL;
