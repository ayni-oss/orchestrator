#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timestamp {
    pub seconds: i64,
    pub nanos: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptKind {
    CreateWorkspace,
    CreateProject,
    CreateConversation,
    SendMessage,
}

impl ReceiptKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::CreateWorkspace => "create_workspace",
            Self::CreateProject => "create_project",
            Self::CreateConversation => "create_conversation",
            Self::SendMessage => "send_message",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "create_workspace" => Some(Self::CreateWorkspace),
            "create_project" => Some(Self::CreateProject),
            "create_conversation" => Some(Self::CreateConversation),
            "send_message" => Some(Self::SendMessage),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresentationState {
    Accepted,
    Queued,
    Processing,
    Completed,
    Failed,
}

impl PresentationState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Queued => "queued",
            Self::Processing => "processing",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "accepted" => Some(Self::Accepted),
            "queued" => Some(Self::Queued),
            "processing" => Some(Self::Processing),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulingState {
    WaitingForPredecessor,
    Eligible,
}

impl SchedulingState {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "waiting_for_predecessor" => Some(Self::WaitingForPredecessor),
            "eligible" => Some(Self::Eligible),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchState {
    NotEligible,
    Pending,
    Stored,
    Transferred,
    Degraded,
}

impl DispatchState {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "not_eligible" => Some(Self::NotEligible),
            "pending" => Some(Self::Pending),
            "stored" => Some(Self::Stored),
            "transferred" => Some(Self::Transferred),
            "degraded" => Some(Self::Degraded),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionState {
    NotStarted,
    Processing,
    Completed,
    Failed,
}

impl ExecutionState {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "not_started" => Some(Self::NotStarted),
            "processing" => Some(Self::Processing),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
}

impl MessageRole {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandReceipt {
    pub command_id: String,
    pub kind: ReceiptKind,
    pub workspace_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub turn_id: String,
    pub accepted_at: Timestamp,
    pub conversation_revision: u64,
    pub resume_cursor: String,
    pub presentation_state: PresentationState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workspace {
    pub id: String,
    pub display_name: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub resource_version: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Project {
    pub id: String,
    pub workspace_id: String,
    pub display_name: String,
    pub goal: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub resource_version: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Conversation {
    pub id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub title: String,
    pub created_at: Timestamp,
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Created<T> {
    pub resource: T,
    pub receipt: CommandReceipt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub role: MessageRole,
    pub content: String,
    pub accepted_at: Timestamp,
    pub sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Turn {
    pub id: String,
    pub conversation_id: String,
    pub input_message_id: String,
    pub sequence: u64,
    pub execution_binding_revision: String,
    pub context_revision: u64,
    pub assignment_id: String,
    pub executor_generation: u64,
    pub scheduling_state: SchedulingState,
    pub dispatch_state: DispatchState,
    pub execution_state: ExecutionState,
    pub response_message_id: String,
    pub failure_reason: String,
    pub accepted_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Activity {
    pub id: String,
    pub conversation_id: String,
    pub conversation_revision: u64,
    pub cursor: String,
    pub turn_id: String,
    pub stage: String,
    pub presentation_state: PresentationState,
    pub waiting_on: String,
    pub safe_reason: String,
    pub accepted_at: Timestamp,
    pub safe_detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalTurnSummary {
    pub id: String,
    pub sequence: u64,
    pub input_message_id: String,
    pub presentation_state: PresentationState,
    pub response_message_id: String,
    pub safe_failure_reason: String,
    pub accepted_at: Timestamp,
    pub terminal_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationSnapshot {
    pub workspace_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub conversation_revision: u64,
    pub recent_messages: Vec<Message>,
    pub nonterminal_turns: Vec<Turn>,
    pub recent_terminal_turns: Vec<TerminalTurnSummary>,
    pub message_count: u64,
    pub turn_count: u64,
    pub has_older_history: bool,
    pub history_snapshot_id: String,
    pub current_activity: Option<Activity>,
    pub resume_cursor: String,
    pub title: String,
    pub created_at: Timestamp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryKind {
    Message,
    TerminalTurn,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryRecord {
    pub sequence: u64,
    pub id: String,
    pub kind: HistoryKind,
    pub role_or_state: String,
    pub content_or_outcome: String,
    pub source_turn_id: String,
    pub accepted_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryPage {
    pub records: Vec<HistoryRecord>,
    pub snapshot_id: String,
    pub next_before_sequence: u64,
    pub has_more: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportDecisionReceipt {
    pub report_source: String,
    pub report_id: String,
    pub decision_event_id: String,
    pub accepted: bool,
    pub safe_reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboxRecord {
    pub id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub subject: String,
    pub event_bytes: Vec<u8>,
}
