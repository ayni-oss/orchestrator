//! Safe classification of rejected transport bytes. Future payloads are retained
//! without interpreting them or granting authority; malformed content is hashed only.

use super::{
    APPLICATION_SOURCE, MAX_COMMAND_BYTES, MAX_DECISION_BYTES, MAX_REPORT_BYTES, decode_command,
    decode_decision, decode_report,
};
use crate::{EventIdentity, validate_identity};
use cloudevents::{AttributesReader, Data, Event, event::SpecVersion};
use sha2::{Digest as _, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsumerRole {
    ApplicationReport,
    ExecutorCommand,
    ExecutorDecision,
}

impl ConsumerRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApplicationReport => "application_report",
            Self::ExecutorCommand => "executor_command",
            Self::ExecutorDecision => "executor_decision",
        }
    }

    const fn maximum(self) -> usize {
        match self {
            Self::ApplicationReport => MAX_REPORT_BYTES,
            Self::ExecutorCommand => MAX_COMMAND_BYTES,
            Self::ExecutorDecision => MAX_DECISION_BYTES,
        }
    }
}

/// Quarantine identity contains no payload or unvalidated transport attributes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedDelivery {
    pub source: String,
    pub id: String,
    pub digest: String,
    pub unsupported_version: bool,
}

/// Return `None` only when the current role's full decoder accepts the bytes.
///
/// A well-formed future envelope retains its bounded exact bytes. It is never
/// interpreted as a current payload or accepted as application state.
#[must_use]
pub fn rejected_delivery(role: ConsumerRole, bytes: &[u8]) -> Option<RejectedDelivery> {
    let accepted = match role {
        ConsumerRole::ApplicationReport => decode_report(bytes).is_ok(),
        ConsumerRole::ExecutorCommand => decode_command(bytes).is_ok(),
        ConsumerRole::ExecutorDecision => decode_decision(bytes).is_ok(),
    };
    if accepted {
        return None;
    }
    let digest = format!("sha256:{:x}", Sha256::digest(bytes));
    let mut rejected = RejectedDelivery {
        source: "transport".into(),
        id: digest.clone(),
        digest,
        unsupported_version: false,
    };
    if bytes.len() > role.maximum() {
        return Some(rejected);
    }
    let Ok(event) = serde_json::from_slice::<Event>(bytes) else {
        return Some(rejected);
    };
    let identity = EventIdentity {
        source: event.source().clone(),
        id: event.id().into(),
    };
    if validate_identity(&identity).is_err() {
        return Some(rejected);
    }
    rejected.source = identity.source;
    rejected.id = identity.id;
    rejected.unsupported_version = future_envelope(role, &event);
    Some(rejected)
}

fn future_envelope(role: ConsumerRole, event: &Event) -> bool {
    if event.specversion() != SpecVersion::V10
        || event.datacontenttype() != Some("application/json")
        || event.time().is_none()
        || super::decode_workflow_metadata(event).is_err()
        || !matches!(event.data(), Some(Data::Json(data)) if data.is_object())
    {
        return false;
    }
    let Some((base_type, version)) = event.ty().rsplit_once(".v") else {
        return false;
    };
    let Ok(version) = version.parse::<u32>() else {
        return false;
    };
    if version <= 1 || version.to_string() != event.ty().rsplit_once(".v").map_or("", |(_, v)| v) {
        return false;
    }
    let schema_base = match role {
        ConsumerRole::ApplicationReport
            if matches!(
                base_type,
                "build.ayni.orchestrator.conversation.turn.started"
                    | "build.ayni.orchestrator.conversation.turn.completed"
                    | "build.ayni.orchestrator.conversation.turn.failed"
            ) =>
        {
            "conversation-turn-report"
        }
        ConsumerRole::ExecutorCommand
            if base_type == "build.ayni.orchestrator.conversation.turn.requested" =>
        {
            "conversation-turn-requested"
        }
        ConsumerRole::ExecutorDecision
            if base_type == "build.ayni.orchestrator.conversation.report.decided" =>
        {
            "conversation-report-decision"
        }
        _ => return false,
    };
    if role != ConsumerRole::ApplicationReport && event.source().as_str() != APPLICATION_SOURCE {
        return false;
    }
    let expected_schema = format!("ayni://orchestrator/schemas/{schema_base}/v{version}");
    if event
        .dataschema()
        .is_none_or(|schema| schema.as_str() != expected_schema)
    {
        return false;
    }
    stable_extensions(role, event)
}

fn stable_extensions(role: ConsumerRole, event: &Event) -> bool {
    let extension = |name| match event.extension(name) {
        Some(cloudevents::event::ExtensionValue::String(value)) => Some(value.as_str()),
        _ => None,
    };
    for name in ["workspaceid", "projectid", "conversationid", "turnid"] {
        if !extension(name).is_some_and(crate::resource_identifier) {
            return false;
        }
    }
    for name in ["assignmentid", "causationid"] {
        if !extension(name).is_some_and(super::simple_identifier) {
            return false;
        }
    }
    if extension("correlationid") != extension("conversationid") {
        return false;
    }
    let subject = if role == ConsumerRole::ExecutorDecision {
        format!("reports/{}", extension("causationid").unwrap_or_default())
    } else {
        format!(
            "conversations/{}/turns/{}",
            extension("conversationid").unwrap_or_default(),
            extension("turnid").unwrap_or_default()
        )
    };
    event.subject() == Some(subject.as_str())
        && extension("executorgeneration")
            .is_some_and(|value| value.parse::<u64>().is_ok_and(|generation| generation > 0))
}
