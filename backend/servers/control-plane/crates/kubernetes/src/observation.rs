//! Shared transport observation for a Job's single owned result Pod.
use crate::execution::RuntimeKubernetes;
use k8s_openapi::api::{
    batch::v1::Job,
    core::v1::{ContainerStatus, Pod},
};
use kube::{Api, api::ListParams};

impl RuntimeKubernetes {
    pub(crate) async fn owned_operation_pod(
        &self,
        name: &str,
        job: &Job,
    ) -> Result<Pod, &'static str> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let list = pods
            .list(
                &ListParams::default()
                    .labels(&format!("ayni.io/runtime-operation={name}"))
                    .limit(2),
            )
            .await
            .map_err(|_| "kubernetes_unavailable")?;
        single_owned_pod(list.items, job)
    }
}

fn single_owned_pod(mut pods: Vec<Pod>, job: &Job) -> Result<Pod, &'static str> {
    if pods.len() != 1 {
        return Err("runtime_pod_identity_invalid");
    }
    let pod = pods.pop().ok_or("runtime_pod_identity_invalid")?;
    if !pod.metadata.owner_references.as_ref().is_some_and(|refs| {
        refs.iter().any(|reference| {
            reference.controller == Some(true) && Some(&reference.uid) == job.metadata.uid.as_ref()
        })
    }) {
        return Err("runtime_pod_identity_invalid");
    }
    Ok(pod)
}

pub fn container_status<'a>(pod: &'a Pod, name: &str) -> Option<&'a ContainerStatus> {
    pod.status
        .as_ref()?
        .container_statuses
        .as_ref()?
        .iter()
        .find(|status| status.name == name)
}

pub fn termination_message(status: &ContainerStatus, require_success: bool) -> Option<&str> {
    let terminated = status.state.as_ref()?.terminated.as_ref()?;
    if require_success && terminated.exit_code != 0 {
        return None;
    }
    terminated
        .message
        .as_deref()
        .filter(|message| message.len() <= 4096)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_pod_must_be_the_only_pod_and_owned_by_the_observed_job()
    -> Result<(), serde_json::Error> {
        let job: Job = serde_json::from_value(serde_json::json!({"metadata":{"uid":"current"}}))?;
        let pod: Pod = serde_json::from_value(
            serde_json::json!({"metadata":{"ownerReferences":[{"apiVersion":"batch/v1","kind":"Job","name":"job","uid":"current","controller":true}]}}),
        )?;
        assert!(single_owned_pod(vec![pod.clone()], &job).is_ok());
        assert!(single_owned_pod(vec![], &job).is_err());
        assert!(single_owned_pod(vec![pod.clone(), pod.clone()], &job).is_err());
        let old: Job = serde_json::from_value(serde_json::json!({"metadata":{"uid":"old"}}))?;
        assert!(single_owned_pod(vec![pod], &old).is_err());
        Ok(())
    }
    #[test]
    fn result_messages_are_bounded_and_success_is_operation_specific()
    -> Result<(), serde_json::Error> {
        let mut status: ContainerStatus = serde_json::from_value(
            serde_json::json!({"name":"build","image":"image","imageID":"id","ready":false,"restartCount":0,"state":{"terminated":{"exitCode":1,"message":"failure"}}}),
        )?;
        assert_eq!(termination_message(&status, false), Some("failure"));
        assert!(termination_message(&status, true).is_none());
        if let Some(terminated) = status
            .state
            .as_mut()
            .and_then(|state| state.terminated.as_mut())
        {
            terminated.message = Some("x".repeat(4097));
        }
        assert!(termination_message(&status, false).is_none());
        Ok(())
    }
}
