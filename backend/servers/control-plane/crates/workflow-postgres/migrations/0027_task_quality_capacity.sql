-- Complete repository reports retain their exact bounded original bytes.
ALTER TABLE workflow_task_quality DROP CONSTRAINT workflow_task_quality_outcome_check;
ALTER TABLE workflow_task_quality ADD CONSTRAINT workflow_task_quality_outcome_check CHECK(octet_length(outcome) BETWEEN 1 AND 2097152);
ALTER TABLE workflow_task_runs DROP CONSTRAINT workflow_task_runs_command_check;
ALTER TABLE workflow_task_runs ADD CONSTRAINT workflow_task_runs_command_check CHECK(octet_length(command) BETWEEN 1 AND 2097152);
