ALTER TABLE public.workflow_connections
    DROP CONSTRAINT workflow_connections_status_check,
    ADD CONSTRAINT workflow_connections_status_check
        CHECK (status IN ('pending', 'ready', 'failed', 'disabled')
            OR (status = 'deleted' AND NOT enabled));

ALTER TABLE public.workflow_runtime_signers
    ADD COLUMN deleted boolean NOT NULL DEFAULT false,
    ADD CONSTRAINT workflow_runtime_signers_deleted_check CHECK (NOT deleted OR NOT enabled);

ALTER TABLE public.workflow_runtime_signer_receipts
    DROP CONSTRAINT workflow_runtime_signer_receipts_kind_check,
    ADD CONSTRAINT workflow_runtime_signer_receipts_kind_check
        CHECK (kind IN ('save', 'enabled', 'provision', 'delete'));
