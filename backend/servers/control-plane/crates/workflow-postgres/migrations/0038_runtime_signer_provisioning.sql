-- Durable public intent only. Private signing material belongs solely to Kubernetes.
CREATE TABLE workflow_runtime_signer_provisioning (
 id text PRIMARY KEY,
 idempotency_key text NOT NULL UNIQUE,
 safe_request text NOT NULL,
 display_name text NOT NULL CHECK (octet_length(display_name) BETWEEN 1 AND 120),
 namespace text NOT NULL CHECK (octet_length(namespace) BETWEEN 1 AND 63 AND namespace ~ '^[a-z0-9]([a-z0-9-]*[a-z0-9])?$'),
 key_id text NOT NULL CHECK (key_id ~ '^[A-Za-z0-9._-]{1,128}$'),
 mode text NOT NULL CHECK (mode IN ('generate','import')),
 import_public_key text NOT NULL CHECK ((mode='generate' AND import_public_key='') OR (mode='import' AND import_public_key ~ '^[0-9a-f]{64}$')),
 secret_name text NOT NULL UNIQUE CHECK (secret_name ~ '^ayni-runtime-signer-[0-9a-f]{32}$'),
 state text NOT NULL CHECK (state IN ('pending','awaiting_material','blocked','completed')),
 safe_reason text NOT NULL CHECK (safe_reason IN ('','signer_provisioning_inputs_required','signer_provisioning_disabled','signer_provisioning_unavailable','signer_provisioning_access_denied','signer_provisioning_conflict')),
 public_key text NOT NULL DEFAULT '',
 secret_uid text NOT NULL DEFAULT '',
 result text NOT NULL DEFAULT '',
 accepted_at_unix_ms bigint NOT NULL CHECK (accepted_at_unix_ms>=0),
 updated_at_unix_ms bigint NOT NULL CHECK (updated_at_unix_ms>=accepted_at_unix_ms),
 CHECK ((state='completed' AND safe_reason='' AND public_key ~ '^[0-9a-f]{64}$' AND secret_uid ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' AND result<>'') OR (state<>'completed' AND public_key='' AND secret_uid='' AND result='')),
 CHECK (state<>'completed' OR mode='generate' OR public_key=import_public_key)
);
ALTER TABLE workflow_runtime_signer_receipts DROP CONSTRAINT workflow_runtime_signer_receipts_kind_check;
ALTER TABLE workflow_runtime_signer_receipts ADD CONSTRAINT workflow_runtime_signer_receipts_kind_check CHECK (kind IN ('save','enabled','provision'));
REVOKE ALL ON workflow_runtime_signer_provisioning FROM PUBLIC;
