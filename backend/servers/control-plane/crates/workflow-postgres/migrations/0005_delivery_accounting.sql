-- Broker sequence is an observation of a stored copy, never ownership authority.
-- A targeted consumer's contiguous ACK floor completes only that decision handoff.
ALTER TABLE workflow_outbox ADD COLUMN stream_sequence bigint NOT NULL DEFAULT 0
    CHECK (stream_sequence >= 0);
