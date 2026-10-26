ALTER TABLE workflow_command_receipts ADD COLUMN traceparent text NOT NULL
    DEFAULT COALESCE(current_setting('ayni.traceparent', true), '');

CREATE INDEX workflow_command_receipts_turn_trace
    ON workflow_command_receipts (conversation_id, turn_id) WHERE kind = 'send_message';

-- A bounded materialized preview, separate from accepted domain transitions.
CREATE TABLE workflow_execution_progress (
    turn_id text PRIMARY KEY REFERENCES workflow_turns(id),
    session_id text NOT NULL,
    generation bigint NOT NULL CHECK (generation > 0),
    sequence bigint NOT NULL CHECK (sequence > 0),
    round integer NOT NULL CHECK (round BETWEEN 0 AND 8),
    call_id text NOT NULL CHECK (octet_length(call_id) <= 128),
    stage text NOT NULL CHECK (stage IN ('preparation','model','tool','scripted')),
    tool_name text NOT NULL CHECK (octet_length(tool_name) <= 128),
    tool_invocation_id text NOT NULL CHECK (octet_length(tool_invocation_id) <= 128),
    provisional_text text NOT NULL CHECK (octet_length(provisional_text) <= 16384),
    stage_observed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    observed_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX workflow_execution_timeline_resources
    ON workflow_execution_timeline USING gin (affected_resource_ids);
