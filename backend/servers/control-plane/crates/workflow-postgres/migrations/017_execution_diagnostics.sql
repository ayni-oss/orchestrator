CREATE TABLE public.workflow_execution_diagnostics (
    cursor bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    kind text NOT NULL CHECK (kind IN ('runtime', 'task', 'session')),
    execution_id text NOT NULL,
    digest text NOT NULL,
    diagnostic text NOT NULL CHECK (octet_length(diagnostic) <= 8192),
    observed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    published_at timestamptz,
    UNIQUE (kind, execution_id, digest)
);
CREATE INDEX workflow_execution_diagnostics_replay
    ON public.workflow_execution_diagnostics (kind, execution_id, cursor);
CREATE INDEX workflow_execution_diagnostics_pending
    ON public.workflow_execution_diagnostics (cursor) WHERE published_at IS NULL;
CREATE INDEX workflow_execution_diagnostics_retention
    ON public.workflow_execution_diagnostics (observed_at, cursor);
