-- Runtime signers: final table definitions for fresh installations.

CREATE TABLE public.workflow_runtime_signer_provisioning (
    id text NOT NULL,
    idempotency_key text NOT NULL,
    safe_request text NOT NULL,
    display_name text NOT NULL,
    namespace text NOT NULL,
    key_id text NOT NULL,
    mode text NOT NULL,
    import_public_key text NOT NULL,
    secret_name text NOT NULL,
    state text NOT NULL,
    safe_reason text NOT NULL,
    public_key text DEFAULT ''::text NOT NULL,
    secret_uid text DEFAULT ''::text NOT NULL,
    result text DEFAULT ''::text NOT NULL,
    accepted_at_unix_ms bigint NOT NULL,
    updated_at_unix_ms bigint NOT NULL,
    CONSTRAINT workflow_runtime_signer_provisioning_accepted_at_unix_ms_check CHECK ((accepted_at_unix_ms >= 0)),
    CONSTRAINT workflow_runtime_signer_provisioning_check CHECK ((((mode = 'generate'::text) AND (import_public_key = ''::text)) OR ((mode = 'import'::text) AND (import_public_key ~ '^[0-9a-f]{64}$'::text)))),
    CONSTRAINT workflow_runtime_signer_provisioning_check1 CHECK ((updated_at_unix_ms >= accepted_at_unix_ms)),
    CONSTRAINT workflow_runtime_signer_provisioning_check2 CHECK ((((state = 'completed'::text) AND (safe_reason = ''::text) AND (public_key ~ '^[0-9a-f]{64}$'::text) AND (secret_uid ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'::text) AND (result <> ''::text)) OR ((state <> 'completed'::text) AND (public_key = ''::text) AND (secret_uid = ''::text) AND (result = ''::text)))),
    CONSTRAINT workflow_runtime_signer_provisioning_check3 CHECK (((state <> 'completed'::text) OR (mode = 'generate'::text) OR (public_key = import_public_key))),
    CONSTRAINT workflow_runtime_signer_provisioning_display_name_check CHECK (((octet_length(display_name) >= 1) AND (octet_length(display_name) <= 120))),
    CONSTRAINT workflow_runtime_signer_provisioning_key_id_check CHECK ((key_id ~ '^[A-Za-z0-9._-]{1,128}$'::text)),
    CONSTRAINT workflow_runtime_signer_provisioning_mode_check CHECK ((mode = ANY (ARRAY['generate'::text, 'import'::text]))),
    CONSTRAINT workflow_runtime_signer_provisioning_namespace_check CHECK ((((octet_length(namespace) >= 1) AND (octet_length(namespace) <= 63)) AND (namespace ~ '^[a-z0-9]([a-z0-9-]*[a-z0-9])?$'::text))),
    CONSTRAINT workflow_runtime_signer_provisioning_safe_reason_check CHECK ((safe_reason = ANY (ARRAY[''::text, 'signer_provisioning_inputs_required'::text, 'signer_provisioning_disabled'::text, 'signer_provisioning_unavailable'::text, 'signer_provisioning_access_denied'::text, 'signer_provisioning_conflict'::text]))),
    CONSTRAINT workflow_runtime_signer_provisioning_secret_name_check CHECK ((secret_name ~ '^ayni-runtime-signer-[0-9a-f]{32}$'::text)),
    CONSTRAINT workflow_runtime_signer_provisioning_state_check CHECK ((state = ANY (ARRAY['pending'::text, 'awaiting_material'::text, 'blocked'::text, 'completed'::text]))),
    CONSTRAINT workflow_runtime_signer_provisioning_idempotency_key_key UNIQUE (idempotency_key),
    CONSTRAINT workflow_runtime_signer_provisioning_pkey PRIMARY KEY (id),
    CONSTRAINT workflow_runtime_signer_provisioning_secret_name_key UNIQUE (secret_name)
);

CREATE TABLE public.workflow_runtime_signers (
    id text NOT NULL,
    display_name text NOT NULL,
    key_id text NOT NULL,
    public_key text NOT NULL,
    secret_name text NOT NULL,
    secret_uid text NOT NULL,
    active_revision text NOT NULL,
    revision bigint NOT NULL,
    enabled boolean NOT NULL,
    CONSTRAINT workflow_runtime_signers_display_name_check CHECK (((octet_length(display_name) >= 1) AND (octet_length(display_name) <= 120))),
    CONSTRAINT workflow_runtime_signers_key_id_check CHECK ((key_id ~ '^[A-Za-z0-9._-]{1,128}$'::text)),
    CONSTRAINT workflow_runtime_signers_public_key_check CHECK ((public_key ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT workflow_runtime_signers_revision_check CHECK ((revision > 0)),
    CONSTRAINT workflow_runtime_signers_secret_name_check CHECK (((octet_length(secret_name) <= 63) AND (secret_name ~ '^ayni-runtime-signer-[a-z0-9]([a-z0-9-]*[a-z0-9])?$'::text))),
    CONSTRAINT workflow_runtime_signers_secret_uid_check CHECK ((secret_uid ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'::text)),
    CONSTRAINT workflow_runtime_signers_pkey PRIMARY KEY (id)
);

CREATE TABLE public.workflow_runtime_signer_revisions (
    revision_id text NOT NULL,
    signer_id text NOT NULL,
    configuration text NOT NULL,
    CONSTRAINT workflow_runtime_signer_revisions_pkey PRIMARY KEY (revision_id),
    CONSTRAINT workflow_runtime_signer_revisions_signer_id_revision_id_key UNIQUE (signer_id, revision_id),
    CONSTRAINT workflow_runtime_signer_revisions_signer_id_fkey FOREIGN KEY (signer_id) REFERENCES public.workflow_runtime_signers(id)
);

CREATE TABLE public.workflow_runtime_signer_receipts (
    idempotency_key text NOT NULL,
    command_id text NOT NULL,
    signer_id text NOT NULL,
    kind text NOT NULL,
    safe_request text NOT NULL,
    result text NOT NULL,
    CONSTRAINT workflow_runtime_signer_receipts_kind_check CHECK ((kind = ANY (ARRAY['save'::text, 'enabled'::text, 'provision'::text]))),
    CONSTRAINT workflow_runtime_signer_receipts_command_id_key UNIQUE (command_id),
    CONSTRAINT workflow_runtime_signer_receipts_pkey PRIMARY KEY (idempotency_key),
    CONSTRAINT workflow_runtime_signer_receipts_signer_id_fkey FOREIGN KEY (signer_id) REFERENCES public.workflow_runtime_signers(id)
);

-- Complete cyclic references after both tables exist. These constraints
-- belong to this layer and are installed in the same migration transaction.

ALTER TABLE public.workflow_runtime_signers
    ADD CONSTRAINT workflow_runtime_signers_active_revision FOREIGN KEY (id, active_revision) REFERENCES public.workflow_runtime_signer_revisions(signer_id, revision_id) DEFERRABLE INITIALLY DEFERRED;
