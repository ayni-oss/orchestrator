-- Safe provider settings and bounded model discovery are separate from credentials.
ALTER TABLE workflow_connections DROP CONSTRAINT workflow_connections_provider_check;
ALTER TABLE workflow_connections DROP CONSTRAINT workflow_connections_capability_check;
ALTER TABLE workflow_connections ADD CONSTRAINT workflow_connections_provider_capability_check CHECK (
 (provider='github' AND capability IN ('source_control','container_registry')) OR
 (provider='openai' AND capability='llm'));
ALTER TABLE workflow_connections ADD COLUMN settings text NOT NULL DEFAULT '{}';
ALTER TABLE workflow_connections ADD COLUMN models text NOT NULL DEFAULT '[]';
ALTER TABLE workflow_agent_configurations ADD COLUMN connection_id text NOT NULL DEFAULT '';
ALTER TABLE workflow_turn_inputs ADD COLUMN credential_revision text NOT NULL DEFAULT '';
ALTER TABLE workflow_sessions ADD COLUMN credential_binding text NOT NULL DEFAULT '';
