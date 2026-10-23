-- Preserve accepted uncertainty evidence after the current publication resolves.
CREATE TABLE workflow_task_publication_reports (
 publication_id text NOT NULL REFERENCES workflow_task_publications(publication_id),
 ordinal smallint NOT NULL CHECK(ordinal BETWEEN 1 AND 2),
 report_digest text NOT NULL CHECK(octet_length(report_digest)=71),
 outcome bytea NOT NULL CHECK(octet_length(outcome) BETWEEN 1 AND 8192),
 generation bigint NOT NULL CHECK(generation>0),
 pod_uid text NOT NULL CHECK(octet_length(pod_uid)<=64),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(publication_id,ordinal),
 UNIQUE(publication_id,report_digest)
);
