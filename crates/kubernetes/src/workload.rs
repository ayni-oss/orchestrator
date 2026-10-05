//! Shared source checkout and launcher support for isolated Kubernetes workloads.
use crate::configuration::{ExecutionConfiguration, WorkloadRuntime};
use serde_json::{Value, json};

pub struct WorkspaceSupport {
    pub init_containers: Vec<Value>,
    pub volumes: Vec<Value>,
}

pub fn workspace_support(name: &str, worker_image: &str) -> WorkspaceSupport {
    WorkspaceSupport {
        init_containers: vec![
            json!({
                "name":"launcher", "image":worker_image, "args":["install"],
                "securityContext":container_security(), "resources":container_resources(),
                "volumeMounts":[{"name":"launcher","mountPath":"/launcher"}]
            }),
            json!({
                "name":"checkout", "image":worker_image, "args":["checkout"], "workingDir":"/workspace",
                "securityContext":container_security(), "resources":container_resources(),
                "env":[{"name":"SOURCE_TOKEN","valueFrom":{"secretKeyRef":{"name":name,"key":"token"}}}],
                "volumeMounts":source_mounts()
            }),
        ],
        volumes: vec![
            json!({"name":"workspace","emptyDir":{"sizeLimit":"4Gi"}}),
            json!({"name":"scratch","emptyDir":{"sizeLimit":"4Gi"}}),
            json!({"name":"launcher","emptyDir":{"sizeLimit":"128Mi"}}),
            json!({"name":"request","configMap":{"name":name}}),
        ],
    }
}

pub fn node_selector(platform: &str, allow_emulation: bool) -> Value {
    if allow_emulation {
        json!({})
    } else {
        json!({"kubernetes.io/os":"linux","kubernetes.io/arch":platform.split('/').nth(1)})
    }
}

pub fn container_security() -> Value {
    json!({
        "runAsNonRoot":true, "runAsUser":10001, "runAsGroup":10001,
        "readOnlyRootFilesystem":true, "allowPrivilegeEscalation":false,
        "capabilities":{"drop":["ALL"]}
    })
}

pub fn container_resources() -> Value {
    json!({
        "requests":{"cpu":"500m","memory":"1Gi"},
        "limits":{"cpu":"2","memory":"4Gi","ephemeral-storage":"8Gi"}
    })
}

fn source_mounts() -> Vec<Value> {
    vec![
        json!({"name":"workspace","mountPath":"/workspace"}),
        json!({"name":"scratch","mountPath":"/scratch"}),
        // Mount the immutable key as a regular file: the worker rejects symlinks.
        json!({"name":"request","mountPath":"/request/execution.json","subPath":"execution.json","readOnly":true}),
    ]
}

pub fn worker_container(operation: &str, image: &str, environment: &[Value]) -> Value {
    let mut mounts = source_mounts();
    mounts.push(json!({"name":"launcher","mountPath":"/launcher","readOnly":true}));
    json!({
        "name":operation, "image":image, "imagePullPolicy":"Always",
        "args":["/launcher/runtime-worker",operation], "workingDir":"/workspace",
        "securityContext":container_security(), "resources":container_resources(),
        "env":environment, "terminationMessagePolicy":"File", "volumeMounts":mounts
    })
}

/// Wraps a source operation with checkout support and its own scoped container.
pub fn source_job(
    name: &str,
    configuration: &ExecutionConfiguration,
    runtime: &WorkloadRuntime,
    service_account: &str,
    container: &Value,
) -> Value {
    let support = workspace_support(name, &configuration.worker_image);
    let pulls: Vec<_> = runtime
        .pull_secret
        .iter()
        .map(|name| json!({"name":name}))
        .collect();
    json!({
        "apiVersion":"batch/v1", "kind":"Job",
        "metadata":{"name":name,"namespace":configuration.namespace,"labels":{"ayni.io/runtime-operation":name}},
        "spec":{"backoffLimit":0,"activeDeadlineSeconds":1800,
            "template":{"metadata":{"labels":{"ayni.io/runtime-operation":name}},"spec":{
                "restartPolicy":"Never", "serviceAccountName":service_account,
                "automountServiceAccountToken":false, "imagePullSecrets":pulls,
                "nodeSelector":node_selector(&runtime.platform, configuration.allow_emulation),
                "securityContext":{"fsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},
                "initContainers":support.init_containers, "containers":[container], "volumes":support.volumes
            }}
        }
    })
}

pub fn runtime_environment() -> Vec<Value> {
    vec![
        json!({"name":"HOME","value":"/scratch"}),
        json!({"name":"TMPDIR","value":"/scratch"}),
        json!({"name":"XDG_CACHE_HOME","value":"/scratch/cache"}),
        json!({"name":"CARGO_HOME","value":"/scratch/cargo"}),
        json!({"name":"MISE_CACHE_DIR","value":"/scratch/mise-cache"}),
        json!({"name":"MISE_STATE_DIR","value":"/scratch/mise-state"}),
    ]
}
