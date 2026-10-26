//! Provisional execution observation. This snapshot grants no authority or completion.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionProgress {
    pub sequence: u64,
    pub round: u32,
    pub call_id: String,
    pub stage: String,
    pub tool_name: String,
    #[serde(default)]
    pub tool_invocation_id: String,
    pub provisional_text: String,
}

impl ExecutionProgress {
    #[must_use]
    pub fn valid(&self) -> bool {
        let identifier = |value: &str| {
            value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        };
        self.sequence > 0
            && i64::try_from(self.sequence).is_ok()
            && self.round <= 8
            && matches!(
                self.stage.as_str(),
                "preparation" | "model" | "tool" | "scripted"
            )
            && identifier(&self.call_id)
            && identifier(&self.tool_name)
            && identifier(&self.tool_invocation_id)
            && self.provisional_text.len() <= 16384
            && !self
                .provisional_text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    }
}
