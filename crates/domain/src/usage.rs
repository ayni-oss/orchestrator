use serde::{Deserialize, Serialize};

/// Stages that issue provider model interactions.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelUsageStage {
    /// The bounded coding-agent loop.
    Coding,
    /// The bounded draft-PR authoring loop.
    PrPreparation,
}

impl ModelUsageStage {
    /// Returns the stable storage and API name for this stage.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Coding => "coding",
            Self::PrPreparation => "pr_preparation",
        }
    }
}

/// Terminal state of one provider interaction, plus the spool-only started state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelInteractionOutcome {
    /// The request was issued but the worker did not retain a terminal response.
    InProgress,
    /// The provider returned a completed response.
    Succeeded,
    /// The interaction ended with an error.
    Failed,
    /// The worker was stopped before the interaction completed.
    Cancelled,
}

impl ModelInteractionOutcome {
    /// Returns the stable storage and API name for this outcome.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Raw token counts reported by a provider for one interaction.
///
/// Every field is independently nullable. Absence is never rewritten to zero.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderTokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

impl ProviderTokenUsage {
    /// Whether the provider supplied at least one token counter.
    #[must_use]
    pub const fn is_available(self) -> bool {
        self.input_tokens.is_some()
            || self.output_tokens.is_some()
            || self.cached_input_tokens.is_some()
            || self.reasoning_tokens.is_some()
            || self.total_tokens.is_some()
    }
}

/// Bounded, prompt-free worker spool record for one provider interaction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelUsageReport {
    pub schema_version: u8,
    pub call_id: String,
    pub stage: ModelUsageStage,
    pub provider: String,
    /// Model identity requested by the task profile.
    pub model: String,
    /// Exact model identity returned by the provider, when supplied.
    pub resolved_model: Option<String>,
    pub started_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub latency_ms: Option<u64>,
    pub outcome: ModelInteractionOutcome,
    pub tokens: ProviderTokenUsage,
}

impl ModelUsageReport {
    pub const SCHEMA_VERSION: u8 = 1;
}

#[cfg(test)]
mod tests {
    use super::ProviderTokenUsage;

    #[test]
    fn unavailable_usage_remains_null_in_json() -> Result<(), serde_json::Error> {
        let value = serde_json::to_value(ProviderTokenUsage::default())?;
        assert!(value["input_tokens"].is_null());
        assert!(value["cached_input_tokens"].is_null());
        assert!(!ProviderTokenUsage::default().is_available());
        Ok(())
    }
}
