-- Raw debug payloads never enter the sanitised diagnostic outbox.
CREATE TABLE public.workflow_log_sources (
    id text PRIMARY KEY,
    source text NOT NULL CHECK (octet_length(source) <= 8192),
    sequence bigint NOT NULL CHECK (sequence >= 0),
    digest text NOT NULL,
    receipts text NOT NULL DEFAULT '{}' CHECK (octet_length(receipts) <= 8192),
    observed_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE public.workflow_log_records (
    cursor bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    source_id text NOT NULL REFERENCES public.workflow_log_sources(id),
    sequence bigint NOT NULL CHECK (sequence > 0),
    digest text NOT NULL,
    record text NOT NULL CHECK (octet_length(record) <= 65536),
    payload_bytes bigint NOT NULL CHECK (payload_bytes >= 0),
    observed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(source_id,sequence)
);
CREATE INDEX workflow_log_records_retention ON public.workflow_log_records(observed_at,cursor);
CREATE TABLE public.workflow_log_retention (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    expired_through bigint NOT NULL DEFAULT 0,
    retained_bytes bigint NOT NULL DEFAULT 0 CHECK (retained_bytes >= 0)
);
INSERT INTO public.workflow_log_retention(singleton) VALUES(true);
