use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{fs, process::Command};

const CHART: &str = "deploy/charts/ayni-orchestrator";
const LOCAL: &str = "dev/charts/ayni-orchestrator-local";

fn helm(args: &[&str], accepted: bool) -> Result<String> {
    let scratch = tempfile::tempdir()?;
    let output = Command::new("helm")
        .args(args)
        .env("HELM_NO_PLUGINS", "1")
        .env(
            "HELM_REPOSITORY_CONFIG",
            scratch.path().join("repositories.yaml"),
        )
        .env("HELM_REPOSITORY_CACHE", scratch.path().join("cache"))
        .env("HELM_PLUGINS", scratch.path().join("plugins"))
        .output()?;
    ensure!(
        output.status.success() == accepted,
        "Helm acceptance mismatch for {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).context("Helm returned non-UTF8")
}

fn render(chart: &str, namespace: &str, values: &[&str], accepted: bool) -> Result<String> {
    let mut args = vec![
        "template",
        "independent-release",
        chart,
        "--namespace",
        namespace,
    ];
    for value in values {
        args.extend(["--set", value]);
    }
    helm(&args, accepted)
}

fn contains(text: &str, yes: &[&str], no: &[&str]) -> Result<()> {
    for required in yes {
        ensure!(text.contains(required), "render is missing {required}");
    }
    for forbidden in no {
        ensure!(
            !text.contains(forbidden),
            "render contains forbidden {forbidden}"
        );
    }
    Ok(())
}

fn document<'a>(text: &'a str, kind: &str, name: &str) -> Result<&'a str> {
    text.split("\n---")
        .find(|doc| {
            doc.contains(&format!("kind: {kind}\n"))
                && doc.lines().any(|line| line == format!("  name: {name}"))
        })
        .with_context(|| format!("missing {kind}/{name}"))
}

fn section<'a>(text: &'a str, start: &str, end: &str) -> Result<&'a str> {
    Ok(text
        .split_once(start)
        .with_context(|| format!("missing {start}"))?
        .1
        .split(end)
        .next()
        .unwrap_or_default())
}

pub fn check() -> Result<()> {
    helm(&["dependency", "build", LOCAL], true)?;
    for chart in [CHART, LOCAL] {
        helm(&["lint", chart], true)?;
    }
    let reusable = render(CHART, "test", &[], true)?;
    reusable_checks(&reusable)?;
    local_checks()?;
    runtime()?;
    provider()?;
    publication()?;
    diagnostics()?;
    println!("Helm role, retention, isolation, debug and runtime contracts passed");
    Ok(())
}

fn reusable_checks(reusable: &str) -> Result<()> {
    contains(
        reusable,
        &[
            "name: workflow-postgres-migrator",
            "name: workflow-postgres-runtime",
            "ORCHESTRATOR_WORKFLOW_MIGRATION_DATABASE_URL_PATH",
            "ORCHESTRATOR_WORKFLOW_DATABASE_URL_PATH",
            "ORCHESTRATOR_WORKFLOW_DATABASE_ROLE: \"ayni_runtime\"",
            "name: workflow-jetstream-application",
            "name: workflow-jetstream-executor",
            "ORCHESTRATOR_WORKFLOW_BROKER_URL: \"nats://jetstream:4222\"",
            "ORCHESTRATOR_EXECUTOR_LEDGER_PATH",
            "helm.sh/resource-policy: keep",
            "ORCHESTRATOR_GRPC_BIND_ADDRESS: \"127.0.0.1:50051\"",
            "name: session-authority",
            "secretName: \"workflow-session-authority\"",
        ],
        &[
            "kind: Secret\n",
            "postgresql://",
            "  nats.conf:",
            "workflow.sqlite",
            "name: web",
            "key: api-key",
            "port: 443",
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        ],
    )?;
    let cp = document(reusable, "Deployment", "control-plane")?;
    let migration = section(cp, "      initContainers:", "      containers:")?;
    let serving = section(cp, "        - name: control-plane\n", "      volumes:")?;
    contains(
        migration,
        &["name: workflow-migrate", "workflow-postgres-migrator"],
        &[
            "workflow-postgres-runtime",
            "jetstream",
            "inspection-scratch",
        ],
    )?;
    contains(
        serving,
        &[
            "readOnlyRootFilesystem: true",
            "name: inspection-scratch",
            "mountPath: /tmp",
            "name: TMPDIR",
            "workflow-postgres-runtime",
            "workflow-jetstream-application",
            "ORCHESTRATOR_SESSION_AUTHORITY_TOKEN_PATH",
        ],
        &["workflow-postgres-migrator", "workflow-jetstream-executor"],
    )?;
    contains(cp, &["sizeLimit: 256Mi"], &[])?;
    let supervisor = document(reusable, "StatefulSet", "conversation-supervisor")?;
    contains(
        supervisor,
        &[
            "workflow-jetstream-executor",
            "ORCHESTRATOR_SESSION_AUTHORITY_URL",
            "ORCHESTRATOR_SESSION_AUTHORITY_TOKEN_PATH",
        ],
        &["postgres", "workflow-jetstream-application"],
    )?;
    for service in reusable
        .split("\n---")
        .filter(|doc| doc.contains("kind: Service\n"))
    {
        contains(service, &[], &["trusted-grpc", "port: 50051"])?;
    }
    Ok(())
}

fn local_checks() -> Result<()> {
    let local = render(LOCAL, "test", &[], true)?;
    contains(
        &local,
        &[
            "postgres:17.6-bookworm@sha256:f3bd19c606e442c3d7bdfa8002e03fe260a1023351e0ea4598032022b68dd6e3",
            "nats:2.15.0-alpine@sha256:ac8f88a6494bffc2c2a5289a0ca61cb28a9145c11ba5677cf24265d07f46d8d4",
            "natsio/nats-box:0.20.0@sha256:d2f04223f40e016f77922a8115d34b0cd5fad2ec10b19b39316ae92d07f3d295",
            "max_payload: 327680",
            "ORCHESTRATOR_WORKFLOW_COMMAND_V1",
            "ORCHESTRATOR_WORKFLOW_REPORT_V1",
            "ORCHESTRATOR_WORKFLOW_DECISION_V1",
            "ORCHESTRATOR_EXECUTION_DIAGNOSTICS_V1",
            "\"max_bytes\": 268435456",
            "\"max_bytes\": 201326592",
            "\"max_bytes\": 67108864",
            "\"discard\": \"new\"",
            "\"deny_delete\": true",
            "\"deny_purge\": true",
            "user: \"workflow-application\"",
            "user: \"workflow-executor\"",
            "user: \"workflow-acceptance\"",
            "user: \"workflow-admin\"",
            "secretKeyRef:",
            "helm.sh/resource-policy: keep",
        ],
        &["ALTER DEFAULT PRIVILEGES", "postgresql://", "name: web"],
    )?;
    for value in [
        "postgres.runtimeSecretName=",
        "postgres.runtimeSecretName=workflow-postgres-migrator",
        "jetstream.executorSecretName=workflow-jetstream-application",
        "controlPlane.httpPort=50051",
        "providers.sessionAuthoritySecretName=workflow-jetstream-executor",
    ] {
        render(CHART, "test", &[value], false)?;
    }
    for value in [
        "postgres.secrets.runtime=workflow-postgres-migrator",
        "postgres.image=postgres:latest",
        "jetstream.serverImage=nats:latest",
        "jetstream.secrets.executor=workflow-jetstream-application",
        "jetstream.secrets.admin=workflow-postgres-admin",
        "ayni-orchestrator.postgres.runtimeSecretName=other-runtime",
    ] {
        render(LOCAL, "test", &[value], false)?;
    }
    Ok(())
}

fn runtime() -> Result<()> {
    let legacy = render(CHART, "test", &["runtimePreparation=null"], true)?;
    contains(
        &legacy,
        &[],
        &[
            "name: runtime-preparation-supervisor",
            "ORCHESTRATOR_RUNTIME_PROFILE_JSON",
        ],
    )?;
    let mut values = vec![
        "runtimePreparation.enabled=true",
        "runtimePreparation.profile.profile_id=rootless-podman-vfs-userns-local-development-v1",
        "runtimePreparation.profile.qualified=false",
        "runtimePreparation.profile.worker_image=local/worker@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "runtimePreparation.profile.executor_image=local/executor@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "runtimePreparation.profile.verifier_image=local/probe@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    ];
    render(CHART, "test", &values, false)?;
    values.push("runtimePreparation.localDevelopment=true");
    contains(
        &render(CHART, "test", &values, true)?,
        &[
            "rootless-podman-vfs-userns-local-development-v1",
            "name: runtime-preparation-supervisor",
        ],
        &[],
    )?;
    values.push("runtimePreparation.profile.qualified=true");
    render(CHART, "test", &values, false)?;
    values.pop();
    signers(&values)?;
    namespace_hash()
}

fn signers(values: &[&str]) -> Result<()> {
    for namespace in ["runtime-live".to_owned(), "a".repeat(54)] {
        for enabled in [false, true] {
            let mut options = if enabled { values.to_vec() } else { vec![] };
            options.push("signerProvisioning.enabled=true");
            let text = render(CHART, &namespace, &options, true)?;
            let expected = format!("{namespace}-runtime");
            let ns = document(&text, "Namespace", &expected)?;
            contains(ns, &["helm.sh/resource-policy: keep"], &[])?;
            ensure!(
                text.matches(&format!("kind: Namespace\nmetadata:\n  name: {expected}\n"))
                    .count()
                    == 1,
                "runtime namespace must render once"
            );
            let role = document(&text, "Role", "runtime-signer-provisioner")?;
            contains(
                role,
                &["verbs: [create, get]", &format!("namespace: {expected}")],
                &["list", "delete", "patch", "update", "watch"],
            )?;
            let cp = document(&text, "Deployment", "control-plane")?;
            contains(
                cp,
                &[
                    "automountServiceAccountToken: false",
                    "expirationSeconds: 600",
                    "defaultMode: 0600",
                ],
                &[],
            )?;
            ensure!(
                cp.matches("mountPath: /var/run/secrets/ayni/signer-provisioning")
                    .count()
                    == 1,
                "signer authority mounted more than once"
            );
            contains(
                section(cp, "      initContainers:", "      containers:")?,
                &[],
                &["/var/run/secrets/ayni/signer-provisioning"],
            )?;
            contains(
                document(&text, "ConfigMap", "control-plane-config")?,
                &[&format!(
                    "ORCHESTRATOR_SIGNER_PROVISIONING_NAMESPACE: \"{expected}\""
                )],
                &["independent-release-runtime"],
            )?;
        }
    }
    for values in [vec![], vec!["signerProvisioning=null"]] {
        contains(
            &render(CHART, "test", &values, true)?,
            &[],
            &[
                "name: runtime-signer-provisioner",
                "mountPath: /var/run/secrets/ayni/signer-provisioning",
            ],
        )?;
    }
    Ok(())
}

fn namespace_hash() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    fs::create_dir(scratch.path().join("templates"))?;
    fs::write(
        scratch.path().join("Chart.yaml"),
        "apiVersion: v2\nname: runtime-namespace-fixture\nversion: 0.1.0\n",
    )?;
    fs::copy(
        format!("{CHART}/templates/_helpers.tpl"),
        scratch.path().join("templates/_helpers.tpl"),
    )?;
    fs::write(
        scratch.path().join("templates/value.yaml"),
        "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: {{ include \"ayni-orchestrator.runtimeNamespace\" . }}\n",
    )?;
    let namespace = "a".repeat(63);
    let expected = format!(
        "{}-{:x}-runtime",
        &namespace[..45],
        Sha256::digest(namespace.as_bytes())
    );
    let expected = format!("{}-{}-runtime", &namespace[..45], &expected[46..54]);
    contains(
        &render(
            scratch.path().to_str().context("chart fixture path")?,
            &namespace,
            &[],
            true,
        )?,
        &[&format!("name: {expected}")],
        &[],
    )
}

fn provider() -> Result<()> {
    let text = render(CHART, "test", &["providers.openai.enabled=true"], true)?;
    contains(
        &text,
        &[
            "ORCHESTRATOR_SESSION_AUTHORITY_TOKEN_PATH",
            "https://api.openai.com/v1",
            "port: 443",
        ],
        &["key: api-key", "kind: Secret\n"],
    )?;
    let fixture = render(
        CHART,
        "test",
        &[
            "providers.openai.enabled=true",
            "providers.openai.fixtureEgress=true",
            "providers.openai.baseUrl=http://provider-fixture.test.svc:8080/v1",
        ],
        true,
    )?;
    contains(&fixture, &["app: provider-fixture"], &["cidr: 0.0.0.0/0"])?;
    for flag in [
        "providers.openai.fixtureEgress=true",
        "providers.openai.fixtureEgress=false",
    ] {
        render(
            CHART,
            "test",
            &[
                "providers.openai.enabled=true",
                flag,
                "providers.openai.baseUrl=http://arbitrary.example/v1",
            ],
            false,
        )?;
    }
    Ok(())
}

fn publication() -> Result<()> {
    let values = [
        "publication.enabled=true",
        "publication.workspaceId=11111111-1111-4111-8111-111111111111",
        "publication.repository=https://github.com/fixture/repo.git",
        "publication.githubSecretName=publication-fixture-github",
    ];
    let text = render(CHART, "test", &values, true)?;
    let publisher = document(&text, "Deployment", "task-publisher")?;
    contains(
        publisher,
        &[
            "ayni-task-publisher",
            "mountPath: /var/run/publication-authority",
            "mountPath: /var/run/publication-github",
            "strategy: { type: Recreate }",
        ],
        &["postgres", "jetstream", "session-authority", "OPENAI"],
    )?;
    contains(&text, &["name: task-publisher-isolation"], &[])?;
    for secret in [
        "workflow-publication-authority",
        "workflow-session-authority",
        "workflow-postgres-runtime",
        "workflow-jetstream-executor",
    ] {
        let value = format!("publication.githubSecretName={secret}");
        let mut options = values.to_vec();
        options.push(&value);
        render(CHART, "test", &options, false)?;
    }
    let mut invalid = values.to_vec();
    invalid.extend([
        "publication.fixtureEnabled=true",
        "publication.fixtureApiUrl=http://unrelated.example:8080",
        "publication.fixtureGitUrl=git://publication-fixture.test.svc:9418/fixture/repo.git",
    ]);
    render(CHART, "test", &invalid, false)?;
    for fixture in [
        "publication.fixtureEnabled=true",
        "publication.fixtureEnabled=false",
    ] {
        let mut options = values.to_vec();
        options.extend([
            "observability.collector.enabled=true",
            fixture,
            "publication.fixtureApiUrl=http://publication-fixture.test.svc:8080",
            "publication.fixtureGitUrl=git://publication-fixture.test.svc:9418/fixture/repo.git",
        ]);
        let traced = render(CHART, "test", &options, true)?;
        let policy = document(&traced, "NetworkPolicy", "task-publisher-isolation")?;
        contains(
            policy,
            &[
                "app.kubernetes.io/name: ayni-workflow-traces",
                "kubernetes.io/metadata.name: test",
            ],
            &[],
        )?;
        ensure!(
            policy.matches("port: 4318").count() == 1,
            "publisher needs exactly one scoped collector egress"
        );
        contains(
            document(&text, "NetworkPolicy", "task-publisher-isolation")?,
            &[],
            &["port: 4318"],
        )?;
        contains(
            document(&traced, "Deployment", "task-publisher")?,
            &["http://workflow-traces.test.svc:4318/v1/traces"],
            &[],
        )?;
    }
    Ok(())
}

fn diagnostics() -> Result<()> {
    let traced = render(
        CHART,
        "test",
        &["observability.collector.enabled=true"],
        true,
    )?;
    contains(
        &traced,
        &[
            "name: workflow-traces",
            "http://workflow-traces.test.svc:4318/v1/traces",
            "otel/opentelemetry-collector-contrib:0.147.0@sha256:e7c92c715f28ff142f3bcaccd4fc5603cf4c71276ef09954a38eb4038500a5a5",
            "emptyDir: { sizeLimit: 32Mi }",
            "name: reader",
            "type: ClusterIP",
            "name: workflow-traces-isolation",
            "automountServiceAccountToken: false",
            "max_megabytes: 8",
            "max_backups: 2",
        ],
        &["type: NodePort", "type: LoadBalancer"],
    )?;
    let config = document(&traced, "ConfigMap", "control-plane-config")?;
    let (metadata, data) = config
        .split_once("\ndata:\n")
        .context("configuration data")?;
    contains(metadata, &[], &["OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"])?;
    contains(
        data,
        &["OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: \"http://workflow-traces.test.svc:4318/v1/traces\""],
        &[],
    )?;
    for enabled in [false, true] {
        let value = format!("debug.enabled={enabled}");
        let text = render(CHART, "test", &[&value], true)?;
        let config = document(&text, "ConfigMap", "control-plane-config")?;
        let (metadata, data) = config
            .split_once("\ndata:\n")
            .context("debug configuration data")?;
        contains(metadata, &[], &["ORCHESTRATOR_"])?;
        contains(data, &[&format!("ORCHESTRATOR_DEBUG: \"{enabled}\"")], &[])?;
        ensure!(
            document(&text, "StatefulSet", "log-collector").is_ok() == enabled,
            "collector enablement mismatch"
        );
        if enabled {
            contains(
                &text,
                &[
                    "resources: [pods/log]",
                    "volumeClaimTemplates:",
                    "ORCHESTRATOR_TELEMETRY_TOKEN_PATH:",
                ],
                &["hostPath:", "kind: ClusterRole\n"],
            )?;
        }
    }
    Ok(())
}
