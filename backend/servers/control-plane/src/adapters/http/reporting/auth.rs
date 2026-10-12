//! Kubernetes validates the dedicated Pod-bound credential; immutable admission
//! data, not the event body, supplies the expected execution bindings.
use super::ReceiptError;
use ayni_orchestrator_events::{EventIdentity, v1::VerificationResultAvailable};
use ayni_orchestrator_runtime::verification::{REPORTING_AUDIENCE, ReportingAdmission};
use k8s_openapi::api::{
    authentication::v1::{TokenReview, TokenReviewSpec, TokenReviewStatus},
    batch::v1::Job,
    core::v1::{ConfigMap, Pod},
};
use kube::{Api, Client, api::PostParams};

#[derive(Clone)]
pub(super) struct KubernetesAuthority {
    pub client: Client,
    pub namespace: String,
}

impl KubernetesAuthority {
    pub async fn authorize(
        &self,
        token: String,
        identity: &EventIdentity,
        payload: &VerificationResultAvailable,
    ) -> Result<(), ReceiptError> {
        let review = TokenReview {
            spec: TokenReviewSpec {
                token: Some(token),
                audiences: Some(vec![REPORTING_AUDIENCE.into()]),
            },
            ..Default::default()
        };
        let review = Api::<TokenReview>::all(self.client.clone())
            .create(&PostParams::default(), &review)
            .await
            .map_err(|_| ReceiptError::Unavailable)?;
        let status = review.status.ok_or(ReceiptError::Unauthorized)?;
        let (pod_name, pod_uid) = pod_identity(&status)?;
        let binding = payload.binding.as_ref().ok_or(ReceiptError::Invalid)?;
        if binding.pod_uid != pod_uid {
            return Err(ReceiptError::Unauthorized);
        }
        let pod = Api::<Pod>::namespaced(self.client.clone(), &self.namespace)
            .get(&pod_name)
            .await
            .map_err(authority_lookup)?;
        let job = Api::<Job>::namespaced(self.client.clone(), &self.namespace)
            .get(&binding.execution_id)
            .await
            .map_err(authority_lookup)?;
        let map = Api::<ConfigMap>::namespaced(self.client.clone(), &self.namespace)
            .get(&binding.execution_id)
            .await
            .map_err(authority_lookup)?;
        validate_authority(
            &self.namespace,
            &status,
            &pod,
            &job,
            &map,
            identity,
            payload,
        )
    }
}

impl KubernetesAuthority {
    pub async fn authorize_operation(
        &self,
        token: String,
        identity: &EventIdentity,
        payload: &ayni_orchestrator_events::v1::RuntimeOperationResultAvailable,
    ) -> Result<(), ReceiptError> {
        let review = TokenReview {
            spec: TokenReviewSpec {
                token: Some(token),
                audiences: Some(vec![REPORTING_AUDIENCE.into()]),
            },
            ..Default::default()
        };
        let review = Api::<TokenReview>::all(self.client.clone())
            .create(&PostParams::default(), &review)
            .await
            .map_err(|_| ReceiptError::Unavailable)?;
        let status = review.status.ok_or(ReceiptError::Unauthorized)?;
        let (name, uid) = pod_identity(&status)?;
        let b = payload.binding.as_ref().ok_or(ReceiptError::Invalid)?;
        let pod = Api::<Pod>::namespaced(self.client.clone(), &self.namespace)
            .get(&name)
            .await
            .map_err(authority_lookup)?;
        let job = Api::<Job>::namespaced(self.client.clone(), &self.namespace)
            .get(&b.execution_id)
            .await
            .map_err(authority_lookup)?;
        let map = Api::<ConfigMap>::namespaced(self.client.clone(), &self.namespace)
            .get(&b.execution_id)
            .await
            .map_err(authority_lookup)?;
        let account = pod
            .spec
            .as_ref()
            .and_then(|s| s.service_account_name.as_deref())
            .ok_or(ReceiptError::Unauthorized)?;
        if b.pod_uid != uid
            || pod.metadata.uid.as_deref() != Some(uid.as_str())
            || status.user.as_ref().and_then(|u| u.username.as_deref())
                != Some(format!("system:serviceaccount:{}:{account}", self.namespace).as_str())
            || job.metadata.uid.is_none()
            || job.metadata.name.as_deref() != Some(b.execution_id.as_str())
            || !pod.metadata.owner_references.as_ref().is_some_and(|refs| {
                refs.iter().any(|o| {
                    o.controller == Some(true)
                        && o.kind == "Job"
                        && o.api_version == "batch/v1"
                        && o.name == b.execution_id
                        && Some(&o.uid) == job.metadata.uid.as_ref()
                })
            })
            || map.immutable != Some(true)
            || map.metadata.name != job.metadata.name
        {
            return Err(ReceiptError::Unauthorized);
        }
        let raw = map
            .data
            .as_ref()
            .and_then(|d| d.get("operation-reporting.json"))
            .filter(|v| v.len() <= ayni_orchestrator_events::MAX_EVENT_BYTES)
            .ok_or(ReceiptError::Unauthorized)?;
        let mut admission: ayni_orchestrator_runtime::operation::OperationAdmission =
            serde_json::from_str(raw).map_err(|_| ReceiptError::Unauthorized)?;
        admission.binding.pod_uid = uid;
        if admission.configuration.source != identity.source
            || admission.binding != *b
            || admission.kind as i32 != payload.kind
        {
            return Err(ReceiptError::Unauthorized);
        }
        Ok(())
    }
}
impl KubernetesAuthority {
    pub async fn authorize_coding(
        &self,
        token: String,
        identity: &EventIdentity,
        payload: &ayni_orchestrator_events::v1::CodingReportAvailable,
    ) -> Result<(), ReceiptError> {
        let review = TokenReview {
            spec: TokenReviewSpec {
                token: Some(token),
                audiences: Some(vec![REPORTING_AUDIENCE.into()]),
            },
            ..Default::default()
        };
        let review = Api::<TokenReview>::all(self.client.clone())
            .create(&PostParams::default(), &review)
            .await
            .map_err(|_| ReceiptError::Unavailable)?;
        let status = review.status.ok_or(ReceiptError::Unauthorized)?;
        let (name, uid) = pod_identity(&status)?;
        let b = payload.binding.as_ref().ok_or(ReceiptError::Invalid)?;
        let pod = Api::<Pod>::namespaced(self.client.clone(), &self.namespace)
            .get(&name)
            .await
            .map_err(authority_lookup)?;
        let job = Api::<Job>::namespaced(self.client.clone(), &self.namespace)
            .get(&b.execution_id)
            .await
            .map_err(authority_lookup)?;
        let map = Api::<ConfigMap>::namespaced(self.client.clone(), &self.namespace)
            .get(&b.execution_id)
            .await
            .map_err(authority_lookup)?;
        let account = pod
            .spec
            .as_ref()
            .and_then(|s| s.service_account_name.as_deref())
            .ok_or(ReceiptError::Unauthorized)?;
        if b.pod_uid != uid
            || pod.metadata.uid.as_deref() != Some(uid.as_str())
            || status.user.as_ref().and_then(|u| u.username.as_deref())
                != Some(format!("system:serviceaccount:{}:{account}", self.namespace).as_str())
            || job.metadata.uid.is_none()
            || job.metadata.name.as_deref() != Some(b.execution_id.as_str())
            || !pod.metadata.owner_references.as_ref().is_some_and(|refs| {
                refs.iter().any(|o| {
                    o.controller == Some(true)
                        && o.kind == "Job"
                        && o.api_version == "batch/v1"
                        && o.name == b.execution_id
                        && Some(&o.uid) == job.metadata.uid.as_ref()
                })
            })
            || map.immutable != Some(true)
            || map.metadata.name != job.metadata.name
        {
            return Err(ReceiptError::Unauthorized);
        }
        let raw = map
            .data
            .as_ref()
            .and_then(|d| d.get("coding-reporting.json"))
            .filter(|v| v.len() <= ayni_orchestrator_events::MAX_EVENT_BYTES)
            .ok_or(ReceiptError::Unauthorized)?;
        let mut admission: ayni_orchestrator_runtime::coding_events::CodingAdmission =
            serde_json::from_str(raw).map_err(|_| ReceiptError::Unauthorized)?;
        admission.binding.pod_uid = uid;
        if admission.configuration.source != identity.source || admission.binding != *b {
            return Err(ReceiptError::Unauthorized);
        }
        Ok(())
    }
}

fn authority_lookup(error: kube::Error) -> ReceiptError {
    match error {
        kube::Error::Api(response) if response.code == 404 => ReceiptError::Unauthorized,
        _ => ReceiptError::Unavailable,
    }
}

fn pod_identity(status: &TokenReviewStatus) -> Result<(String, String), ReceiptError> {
    if status.authenticated != Some(true)
        || status.error.is_some()
        || !status
            .audiences
            .as_ref()
            .is_some_and(|audiences| audiences.iter().any(|a| a == REPORTING_AUDIENCE))
    {
        return Err(ReceiptError::Unauthorized);
    }
    let extra = status
        .user
        .as_ref()
        .and_then(|user| user.extra.as_ref())
        .ok_or(ReceiptError::Unauthorized)?;
    let single = |key| {
        extra
            .get(key)
            .filter(|values| values.len() == 1)
            .and_then(|values| values.first())
            .cloned()
            .ok_or(ReceiptError::Unauthorized)
    };
    Ok((
        single("authentication.kubernetes.io/pod-name")?,
        single("authentication.kubernetes.io/pod-uid")?,
    ))
}

#[allow(
    clippy::too_many_arguments,
    reason = "explicit independent authentication and admission inputs"
)]
fn validate_authority(
    namespace: &str,
    status: &TokenReviewStatus,
    pod: &Pod,
    job: &Job,
    map: &ConfigMap,
    identity: &EventIdentity,
    payload: &VerificationResultAvailable,
) -> Result<(), ReceiptError> {
    let (name, uid) = pod_identity(status)?;
    let binding = payload.binding.as_ref().ok_or(ReceiptError::Invalid)?;
    let account = pod
        .spec
        .as_ref()
        .and_then(|spec| spec.service_account_name.as_deref())
        .ok_or(ReceiptError::Unauthorized)?;
    let username = format!("system:serviceaccount:{namespace}:{account}");
    if status.user.as_ref().and_then(|u| u.username.as_deref()) != Some(username.as_str())
        || pod.metadata.name.as_deref() != Some(name.as_str())
        || pod.metadata.uid.as_deref() != Some(uid.as_str())
        || binding.pod_uid != uid
        || job.metadata.name.as_deref() != Some(binding.execution_id.as_str())
        || job.metadata.uid.is_none()
        || !pod
            .metadata
            .owner_references
            .as_ref()
            .is_some_and(|owners| {
                owners.iter().any(|owner| {
                    owner.kind == "Job"
                        && owner.api_version == "batch/v1"
                        && owner.controller == Some(true)
                        && owner.name == binding.execution_id
                        && Some(&owner.uid) == job.metadata.uid.as_ref()
                })
            })
        || map.immutable != Some(true)
        || map.metadata.name != job.metadata.name
    {
        return Err(ReceiptError::Unauthorized);
    }
    let raw = map
        .data
        .as_ref()
        .and_then(|data| data.get("reporting.json"))
        .filter(|raw| raw.len() <= ayni_orchestrator_events::MAX_EVENT_BYTES)
        .ok_or(ReceiptError::Unauthorized)?;
    let mut admission: ReportingAdmission =
        serde_json::from_str(raw).map_err(|_| ReceiptError::Unauthorized)?;
    admission.binding.pod_uid = uid;
    if admission.configuration.source != identity.source || admission.binding != *binding {
        return Err(ReceiptError::Unauthorized);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn pod_bound_review_requires_audience_owner_and_exact_admission()
    -> Result<(), Box<dyn std::error::Error>> {
        let (identity, payload) = ayni_orchestrator_events::decode(include_bytes!(
            "../../../../../../../proto/testdata/verification/report.json"
        ))?;
        let binding = payload.binding.clone().ok_or("binding")?;
        let status: TokenReviewStatus = serde_json::from_value(
            json!({"authenticated":true,"audiences":[REPORTING_AUDIENCE],"user":{"username":"system:serviceaccount:test:runtime-coding","extra":{"authentication.kubernetes.io/pod-name":["pod-1"],"authentication.kubernetes.io/pod-uid":[binding.pod_uid]}}}),
        )?;
        let pod: Pod = serde_json::from_value(
            json!({"metadata":{"name":"pod-1","uid":binding.pod_uid,"ownerReferences":[{"apiVersion":"batch/v1","kind":"Job","name":binding.execution_id,"uid":"job-uid","controller":true}]},"spec":{"containers":[],"serviceAccountName":"runtime-coding"}}),
        )?;
        let job: Job = serde_json::from_value(
            json!({"metadata":{"name":binding.execution_id,"uid":"job-uid"}}),
        )?;
        let admission = ReportingAdmission {
            configuration: ayni_orchestrator_runtime::verification::ReportingConfiguration {
                source: identity.source.clone(),
                endpoint: "http://control-plane/events/verification".into(),
            },
            binding,
        };
        let map: ConfigMap = serde_json::from_value(
            json!({"metadata":{"name":job.metadata.name},"immutable":true,"data":{"reporting.json":serde_json::to_string(&admission)?}}),
        )?;
        assert!(validate_authority("test", &status, &pod, &job, &map, &identity, &payload).is_ok());
        let mut bad = status.clone();
        bad.audiences = Some(vec!["kubernetes".into()]);
        assert!(validate_authority("test", &bad, &pod, &job, &map, &identity, &payload).is_err());
        let mut bad = pod.clone();
        bad.metadata.owner_references = None;
        assert!(
            validate_authority("test", &status, &bad, &job, &map, &identity, &payload).is_err()
        );
        let mut bad = payload.clone();
        bad.binding.as_mut().ok_or("binding")?.preparation_id = "other".into();
        assert!(validate_authority("test", &status, &pod, &job, &map, &identity, &bad).is_err());
        let mut bad = status;
        bad.user.as_mut().ok_or("user")?.username =
            Some("system:serviceaccount:other:runtime-coding".into());
        assert!(validate_authority("test", &bad, &pod, &job, &map, &identity, &payload).is_err());
        Ok(())
    }
}
