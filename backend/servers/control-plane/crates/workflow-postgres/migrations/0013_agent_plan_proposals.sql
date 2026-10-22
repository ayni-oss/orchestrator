-- Structured agent drafts share immutable plan review without approval authority.
ALTER TABLE workflow_plan_versions DROP CONSTRAINT workflow_plan_versions_author_check;
ALTER TABLE workflow_plan_versions ADD CONSTRAINT workflow_plan_versions_author_check
    CHECK (author IN ('local-operator', 'agent'));

-- Bound the complete signed report including proposal bytes and envelope encoding.
ALTER TABLE workflow_report_receipts DROP CONSTRAINT workflow_report_receipts_exact_report_bytes_check;
ALTER TABLE workflow_report_receipts ADD CHECK (octet_length(exact_report_bytes) <= 131072);
ALTER TABLE workflow_transport_rejections DROP CONSTRAINT workflow_transport_rejections_exact_report_bytes_check;
ALTER TABLE workflow_transport_rejections ADD CHECK (octet_length(exact_report_bytes) <= 131072);
ALTER TABLE workflow_transport_quarantine DROP CONSTRAINT workflow_transport_quarantine_exact_bytes_check;
ALTER TABLE workflow_transport_quarantine ADD CHECK (octet_length(exact_bytes) <= 131072);
ALTER TABLE workflow_pending_reports DROP CONSTRAINT workflow_pending_reports_exact_bytes_check;
ALTER TABLE workflow_pending_reports ADD CHECK (octet_length(exact_bytes) <= 131072);

ALTER TABLE workflow_report_receipts ADD UNIQUE
    (report_source, report_id, report_digest, workspace_id, project_id, conversation_id,
     turn_id, assignment_id, executor_generation);
ALTER TABLE workflow_provider_calls ADD UNIQUE (turn_id, call_id);
ALTER TABLE workflow_context_manifests ADD UNIQUE (turn_id, digest);

CREATE TABLE workflow_plan_proposals (
    version_id text PRIMARY KEY,
    plan_id text NOT NULL,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    source_turn_id text NOT NULL UNIQUE,
    configuration_revision_id text NOT NULL,
    context_digest text NOT NULL,
    provider_call_id text NOT NULL,
    provider_response_id text NOT NULL CHECK (octet_length(provider_response_id) BETWEEN 1 AND 256),
    assignment_id text NOT NULL,
    executor_generation bigint NOT NULL CHECK (executor_generation > 0),
    report_source text NOT NULL,
    report_id text NOT NULL,
    report_digest text NOT NULL,
    invocation_id text NOT NULL CHECK (octet_length(invocation_id) BETWEEN 1 AND 128),
    assumptions text[] NOT NULL CHECK (cardinality(assumptions) <= 16),
    evidence text[] NOT NULL CHECK (cardinality(evidence) <= 16),
    UNIQUE (report_source, report_id),
    FOREIGN KEY (workspace_id, project_id, conversation_id, plan_id, version_id)
        REFERENCES workflow_plan_versions(workspace_id, project_id, conversation_id, plan_id, id),
    FOREIGN KEY (workspace_id, configuration_revision_id)
        REFERENCES workflow_agent_configurations(workspace_id, revision_id),
    FOREIGN KEY (source_turn_id, context_digest) REFERENCES workflow_context_manifests(turn_id, digest),
    FOREIGN KEY (source_turn_id, provider_call_id) REFERENCES workflow_provider_calls(turn_id, call_id),
    FOREIGN KEY (report_source, report_id, report_digest, workspace_id, project_id, conversation_id,
                 source_turn_id, assignment_id, executor_generation)
        REFERENCES workflow_report_receipts(report_source, report_id, report_digest,
                 workspace_id, project_id, conversation_id, turn_id, assignment_id, executor_generation)
);
