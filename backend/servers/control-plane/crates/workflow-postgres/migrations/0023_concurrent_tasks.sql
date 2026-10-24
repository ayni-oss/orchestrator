-- Independent isolated attempts may occupy up to four supervisor slots.
-- Per-task pending uniqueness continues to fence competing continuations.
DROP INDEX workflow_task_runs_one_active;
ALTER TABLE workflow_task_runs ADD COLUMN reconciled_at timestamptz NOT NULL DEFAULT clock_timestamp();
CREATE INDEX workflow_task_runs_reconciliation ON workflow_task_runs(reconciled_at,attempt_id) WHERE state IN ('claimed','reporting','cancelling');
