CREATE TABLE workflow_watch_watermarks (
    conversation_id text PRIMARY KEY REFERENCES workflow_conversations(id),
    pruned_through_cursor bigint NOT NULL DEFAULT 0 CHECK (pruned_through_cursor >= 0)
);

CREATE INDEX workflow_updates_retention ON workflow_updates (accepted_at, conversation_id, cursor);
