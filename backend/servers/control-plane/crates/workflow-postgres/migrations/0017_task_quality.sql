-- Quality evidence is immutable and remains separate from the coding result.
CREATE TABLE workflow_task_quality (
 attempt_id text PRIMARY KEY REFERENCES workflow_task_runs(attempt_id),
 outcome bytea NOT NULL CHECK(octet_length(outcome) BETWEEN 1 AND 131072),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
