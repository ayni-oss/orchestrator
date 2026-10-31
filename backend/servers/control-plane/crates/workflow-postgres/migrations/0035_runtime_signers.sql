-- Public signer configuration references admin-owned immutable Kubernetes objects.
-- No signing material is retained in the database.
CREATE TABLE workflow_runtime_signers (
 id text PRIMARY KEY,
 display_name text NOT NULL CHECK (octet_length(display_name) BETWEEN 1 AND 120),
 key_id text NOT NULL CHECK (key_id ~ '^[A-Za-z0-9._-]{1,128}$'),
 public_key text NOT NULL CHECK (public_key ~ '^[0-9a-f]{64}$'),
 secret_name text NOT NULL CHECK (octet_length(secret_name) <= 63 AND secret_name ~ '^ayni-runtime-signer-[a-z0-9]([a-z0-9-]*[a-z0-9])?$'),
 secret_uid text NOT NULL CHECK (secret_uid ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'),
 active_revision text NOT NULL,
 revision bigint NOT NULL CHECK (revision > 0),
 enabled boolean NOT NULL
);
CREATE TABLE workflow_runtime_signer_revisions (
 revision_id text PRIMARY KEY,
 signer_id text NOT NULL REFERENCES workflow_runtime_signers(id),
 configuration text NOT NULL,
 UNIQUE(signer_id,revision_id)
);
ALTER TABLE workflow_runtime_signers ADD CONSTRAINT workflow_runtime_signers_active_revision
 FOREIGN KEY(id,active_revision) REFERENCES workflow_runtime_signer_revisions(signer_id,revision_id)
 DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE workflow_runtime_signer_receipts (
 idempotency_key text PRIMARY KEY,
 command_id text NOT NULL UNIQUE,
 signer_id text NOT NULL REFERENCES workflow_runtime_signers(id),
 kind text NOT NULL CHECK (kind IN ('save','enabled')),
 safe_request text NOT NULL,
 result text NOT NULL
);
REVOKE ALL ON workflow_runtime_signers,workflow_runtime_signer_revisions,workflow_runtime_signer_receipts FROM PUBLIC;
