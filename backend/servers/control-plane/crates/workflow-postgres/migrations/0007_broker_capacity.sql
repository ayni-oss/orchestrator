-- Broker occupancy is a conservative admission observation, never workflow authority.
CREATE TABLE workflow_broker_capacity (
    lane text PRIMARY KEY CHECK (lane IN ('command','report','decision')),
    retained_messages bigint NOT NULL CHECK (retained_messages >= 0),
    retained_bytes bigint NOT NULL CHECK (retained_bytes >= 0),
    observed_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
