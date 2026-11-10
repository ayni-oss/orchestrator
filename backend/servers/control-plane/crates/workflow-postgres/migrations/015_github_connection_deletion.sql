ALTER TABLE public.workflow_connections
    DROP CONSTRAINT workflow_connections_status_check,
    ADD CONSTRAINT workflow_connections_status_check
        CHECK (status IN ('pending', 'ready', 'failed', 'disabled')
            OR (status = 'deleted' AND capability = 'source_control' AND NOT enabled));

ALTER TABLE public.workflow_connection_operations
    DROP CONSTRAINT workflow_connection_operations_kind_check,
    ADD CONSTRAINT workflow_connection_operations_kind_check
        CHECK (kind IN ('save', 'enable', 'delete'));
