-- Accepted execution history outlives the bounded browser replay window.
CREATE TABLE workflow_execution_timeline (
    activity_id text PRIMARY KEY,
    workspace_id text NOT NULL,
    project_id text NOT NULL,
    conversation_id text NOT NULL,
    conversation_revision bigint NOT NULL CHECK (conversation_revision > 0),
    turn_id text NOT NULL,
    stage text NOT NULL,
    presentation_state text NOT NULL,
    waiting_on text NOT NULL,
    safe_reason text NOT NULL,
    safe_detail text NOT NULL CHECK (octet_length(safe_detail) <= 8192),
    accepted_at timestamptz NOT NULL,
    kind text NOT NULL,
    affected_resource_ids text[] NOT NULL,
    UNIQUE (conversation_id, conversation_revision),
    FOREIGN KEY (workspace_id, project_id, conversation_id)
        REFERENCES workflow_conversations(workspace_id, project_id, id)
);

-- Preserve only facts still retained at installation. Pruned history cannot be reconstructed.
INSERT INTO workflow_execution_timeline
SELECT a.id, a.workspace_id, a.project_id, a.conversation_id,
       a.conversation_revision, a.turn_id, a.stage, a.presentation_state,
       a.waiting_on, a.safe_reason, a.safe_detail, a.accepted_at,
       u.kind, u.affected_resource_ids
FROM workflow_activities a JOIN workflow_updates u ON u.activity_id = a.id;

CREATE FUNCTION workflow_retain_execution_activity() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
BEGIN
    INSERT INTO public.workflow_execution_timeline
    SELECT a.id, a.workspace_id, a.project_id, a.conversation_id,
           a.conversation_revision, a.turn_id, a.stage, a.presentation_state,
           a.waiting_on, a.safe_reason, a.safe_detail, a.accepted_at,
           NEW.kind, NEW.affected_resource_ids
    FROM public.workflow_activities a
    WHERE a.id = NEW.activity_id
      AND a.workspace_id = NEW.workspace_id AND a.project_id = NEW.project_id
      AND a.conversation_id = NEW.conversation_id
      AND a.conversation_revision = NEW.conversation_revision;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'execution activity scope mismatch';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION workflow_retain_execution_activity() FROM PUBLIC;

CREATE TRIGGER workflow_retain_execution_activity
AFTER INSERT ON workflow_updates
FOR EACH ROW EXECUTE FUNCTION workflow_retain_execution_activity();
