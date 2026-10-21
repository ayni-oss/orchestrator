//! Retained prior-schema authority survives forward upgrades and failed upgrades.
use ayni_orchestrator_workflow_postgres::{
    CURRENT_SCHEMA_VERSION, MigrationError, migrate_client, migrate_client_for_runtime,
    validate_client_for_runtime,
};
use sha2::{Digest as _, Sha256};
use std::{error::Error, fmt::Write as _};
use tokio_postgres::{Client, Config, NoTls};
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;
const ORIGINAL: [&str; 2] = [
    include_str!("../migrations/0001_durable_foundation.sql"),
    include_str!("../migrations/0002_report_acceptance.sql"),
];

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL acceptance fixture"]
async fn prior_runtime_schemas_upgrade_without_rewriting_exact_authority() -> TestResult {
    let admin_url = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let runtime_url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    let runtime_role = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?;
    let (admin, admin_driver) = open(admin_url.parse()?).await?;
    for prior in [1, 2] {
        let database = format!("upgrade_v{prior}_{}", Uuid::now_v7().simple());
        // Name is generated locally from ASCII UUID bytes; no supplied SQL enters
        // identifiers. These databases live only inside the disposable container.
        admin
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await?;
        let quoted_role: String = admin
            .query_one("SELECT quote_ident($1)", &[&runtime_role])
            .await?
            .get(0);
        admin.batch_execute(&format!("REVOKE ALL ON DATABASE {database} FROM PUBLIC; GRANT CONNECT ON DATABASE {database} TO {quoted_role}")).await?;
        let mut config: Config = admin_url.parse()?;
        config.dbname(&database);
        let (mut client, driver) = open(config).await?;
        seed_prior_schema(&mut client, prior).await?;
        let before = immutable_evidence(&client, prior).await?;
        if prior == 2 {
            client
                .batch_execute(
                    "CREATE TABLE workflow_transport_quarantine (conflicting_fixture boolean)",
                )
                .await?;
            assert!(matches!(
                migrate_client(&mut client).await,
                Err(MigrationError::Database(_))
            ));
            let version: i64 = client
                .query_one("SELECT max(version) FROM workflow_schema_migrations", &[])
                .await?
                .get(0);
            assert_eq!(
                version, 2,
                "failed forward migration changed schema authority"
            );
            assert_eq!(immutable_evidence(&client, prior).await?, before);
            client
                .batch_execute("DROP TABLE workflow_transport_quarantine")
                .await?;
        }
        migrate_client_for_runtime(&mut client, &runtime_role).await?;
        assert_eq!(immutable_evidence(&client, prior).await?, before);
        let version: i64 = client
            .query_one("SELECT max(version) FROM workflow_schema_migrations", &[])
            .await?
            .get(0);
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        migrate_client_for_runtime(&mut client, &runtime_role).await?;
        assert_eq!(
            immutable_evidence(&client, prior).await?,
            before,
            "repeat migration rewrote retained authority"
        );
        let mut config: Config = runtime_url.parse()?;
        config.dbname(&database);
        let (runtime, runtime_driver) = open(config).await?;
        validate_client_for_runtime(&runtime, &runtime_role).await?;
        assert_eq!(immutable_evidence(&runtime, prior).await?, before);
        drop(runtime);
        runtime_driver.abort();
        drop(client);
        driver.abort();
        admin
            .batch_execute(&format!("DROP DATABASE {database} WITH (FORCE)"))
            .await?;
        println!(
            "prior-schema-upgrade: from={prior} to={CURRENT_SCHEMA_VERSION} immutable-receipts=pass exact-command-outbox=pass report-decision=pass repeat=pass runtime-role=pass"
        );
    }
    admin_driver.abort();
    Ok(())
}

async fn open(config: Config) -> TestResult<(Client, tokio::task::JoinHandle<()>)> {
    let (client, connection) = config.connect(NoTls).await?;
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok((client, driver))
}

async fn seed_prior_schema(client: &mut Client, prior: usize) -> TestResult {
    client.batch_execute("CREATE TABLE workflow_schema_migrations (version bigint PRIMARY KEY CHECK(version>0), description text NOT NULL, checksum text NOT NULL, applied_at timestamptz NOT NULL DEFAULT clock_timestamp())").await?;
    for (index, migration) in ORIGINAL.iter().take(prior).enumerate() {
        let transaction = client.transaction().await?;
        transaction.batch_execute(migration).await?;
        let version = i64::try_from(index + 1)?;
        transaction.execute("INSERT INTO workflow_schema_migrations(version,description,checksum) VALUES ($1,'retained prior runtime',$2)", &[&version, &checksum(migration)]).await?;
        transaction.commit().await?;
    }
    client.batch_execute(SEED).await?;
    if prior == 2 {
        client.batch_execute(SEED_REPORT).await?;
    }
    Ok(())
}

async fn immutable_evidence(client: &Client, prior: usize) -> TestResult<Vec<String>> {
    let mut evidence = Vec::new();
    for table in [
        "workflow_command_receipts",
        "workflow_dispatch_intents",
        "workflow_outbox",
        "workflow_messages",
        "workflow_turns",
        "workflow_activities",
        "workflow_updates",
    ] {
        let row = client.query_one(&format!("SELECT jsonb_agg(to_jsonb(t)-'stream_sequence' ORDER BY to_jsonb(t)::text)::text FROM {table} t"), &[]).await?;
        evidence.push(row.get(0));
    }
    if prior == 2 {
        evidence.push(client.query_one("SELECT jsonb_agg(to_jsonb(t) ORDER BY report_id)::text FROM workflow_report_receipts t", &[]).await?.get(0));
    }
    Ok(evidence)
}

fn checksum(sql: &str) -> String {
    let mut value = String::from("sha256:");
    for byte in Sha256::digest(sql.as_bytes()) {
        let _ = write!(value, "{byte:02x}");
    }
    value
}

const SEED: &str = r"
INSERT INTO workflow_workspaces(id,display_name) VALUES ('018f0f7b-6a4d-7a11-8d77-010203040501','Retained workspace');
INSERT INTO workflow_projects(id,workspace_id,display_name,goal) VALUES ('018f0f7b-6a4d-7a11-8d77-010203040502','018f0f7b-6a4d-7a11-8d77-010203040501','Retained project','Retained goal');
INSERT INTO workflow_conversations(id,workspace_id,project_id,title,revision,next_turn_sequence,next_history_sequence) VALUES ('018f0f7b-6a4d-7a11-8d77-010203040503','018f0f7b-6a4d-7a11-8d77-010203040501','018f0f7b-6a4d-7a11-8d77-010203040502','Retained conversation',2,2,2);
BEGIN;
INSERT INTO workflow_messages(id,workspace_id,project_id,conversation_id,turn_id,role,content,sequence) VALUES ('018f0f7b-6a4d-7a11-8d77-010203040504','018f0f7b-6a4d-7a11-8d77-010203040501','018f0f7b-6a4d-7a11-8d77-010203040502','018f0f7b-6a4d-7a11-8d77-010203040503','018f0f7b-6a4d-7a11-8d77-010203040505','user','immutable admission bytes',1);
INSERT INTO workflow_turns(id,workspace_id,project_id,conversation_id,input_message_id,sequence,execution_binding_revision,context_revision,assignment_id,executor_generation,scheduling_state,dispatch_state,execution_state) VALUES ('018f0f7b-6a4d-7a11-8d77-010203040505','018f0f7b-6a4d-7a11-8d77-010203040501','018f0f7b-6a4d-7a11-8d77-010203040502','018f0f7b-6a4d-7a11-8d77-010203040503','018f0f7b-6a4d-7a11-8d77-010203040504',1,'scripted-v1',2,'018f0f7b-6a4d-7a11-8d77-010203040506',1,'eligible','pending','not_started');
COMMIT;
INSERT INTO workflow_dispatch_intents(turn_id,workspace_id,project_id,conversation_id,eligible,command_event_id,exact_command_bytes) VALUES ('018f0f7b-6a4d-7a11-8d77-010203040505','018f0f7b-6a4d-7a11-8d77-010203040501','018f0f7b-6a4d-7a11-8d77-010203040502','018f0f7b-6a4d-7a11-8d77-010203040503',true,'018f0f7b-6a4d-7a11-8d77-010203040507',decode('0065786163742d636f6d6d616e6400','hex'));
INSERT INTO workflow_outbox(id,workspace_id,project_id,conversation_id,turn_id,subject,event_bytes) SELECT command_event_id,workspace_id,project_id,conversation_id,turn_id,'build.ayni.commands.scripted-v1',exact_command_bytes FROM workflow_dispatch_intents;
INSERT INTO workflow_command_receipts(command_id,principal,method,parent_scope,idempotency_key,request_digest,kind,workspace_id,project_id,conversation_id,message_id,turn_id,conversation_revision,resume_cursor,presentation_state) VALUES ('018f0f7b-6a4d-7a11-8d77-010203040508','local-operator','SendMessage','retained-scope','retained-key',decode(repeat('01',32),'hex'),'send_message','018f0f7b-6a4d-7a11-8d77-010203040501','018f0f7b-6a4d-7a11-8d77-010203040502','018f0f7b-6a4d-7a11-8d77-010203040503','018f0f7b-6a4d-7a11-8d77-010203040504','018f0f7b-6a4d-7a11-8d77-010203040505',2,'workflow-cursor-v1:018f0f7b-6a4d-7a11-8d77-010203040503:1','accepted');
INSERT INTO workflow_activities(id,workspace_id,project_id,conversation_id,conversation_revision,turn_id,stage,presentation_state) VALUES ('018f0f7b-6a4d-7a11-8d77-010203040509','018f0f7b-6a4d-7a11-8d77-010203040501','018f0f7b-6a4d-7a11-8d77-010203040502','018f0f7b-6a4d-7a11-8d77-010203040503',2,'018f0f7b-6a4d-7a11-8d77-010203040505','admission','accepted');
INSERT INTO workflow_updates(id,activity_id,workspace_id,project_id,conversation_id,conversation_revision,kind,affected_resource_ids,presentation_state) VALUES ('018f0f7b-6a4d-7a11-8d77-010203040510','018f0f7b-6a4d-7a11-8d77-010203040509','018f0f7b-6a4d-7a11-8d77-010203040501','018f0f7b-6a4d-7a11-8d77-010203040502','018f0f7b-6a4d-7a11-8d77-010203040503',2,'message_admitted',ARRAY['018f0f7b-6a4d-7a11-8d77-010203040504'],'accepted');
";
const SEED_REPORT: &str = r"
INSERT INTO workflow_report_receipts(report_source,report_id,report_digest,exact_report_bytes,workspace_id,project_id,conversation_id,turn_id,assignment_id,executor_generation,report_sequence,executor_instance_id,decision,safe_reason,decision_event_id,decision_bytes) VALUES ('retained-source','018f0f7b-6a4d-7a11-8d77-010203040511','sha256:'||repeat('0',64),decode('0065786163742d7265706f727400','hex'),'018f0f7b-6a4d-7a11-8d77-010203040501','018f0f7b-6a4d-7a11-8d77-010203040502','018f0f7b-6a4d-7a11-8d77-010203040503','018f0f7b-6a4d-7a11-8d77-010203040505','018f0f7b-6a4d-7a11-8d77-010203040506',1,1,'retained-executor','accepted','','018f0f7b-6a4d-7a11-8d77-010203040512',decode('0065786163742d6465636973696f6e00','hex'));
INSERT INTO workflow_outbox(id,workspace_id,project_id,conversation_id,turn_id,subject,event_bytes) SELECT decision_event_id,workspace_id,project_id,conversation_id,turn_id,'build.ayni.decisions.retained',decision_bytes FROM workflow_report_receipts;
";
