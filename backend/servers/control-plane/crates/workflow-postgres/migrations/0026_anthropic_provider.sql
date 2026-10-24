-- Additional provider uses the existing retained context, budget and uncertainty authority.
ALTER TABLE workflow_agent_configurations DROP CONSTRAINT workflow_agent_configurations_provider_check;
ALTER TABLE workflow_agent_configurations ADD CONSTRAINT workflow_agent_configurations_provider_check CHECK(provider IN ('scripted','openai','anthropic'));
