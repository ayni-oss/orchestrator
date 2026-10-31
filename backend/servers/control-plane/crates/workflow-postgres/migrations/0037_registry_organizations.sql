-- Current connection scope is an organization; immutable receipts and runtime
-- snapshots retain their historical package destinations. Increment revisions
-- so pre-upgrade validations and runtime selections cannot activate stale intent.
UPDATE workflow_connections
SET settings = jsonb_set(settings::jsonb, '{registry_destination}',
        to_jsonb('ghcr.io/' || split_part(settings::jsonb->>'registry_destination', '/', 2)))::text,
    revision = revision + 1
WHERE provider = 'github' AND capability = 'container_registry'
  AND settings::jsonb->>'registry_destination' LIKE 'ghcr.io/%/%';
