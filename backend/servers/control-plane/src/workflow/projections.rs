use super::*;
pub(super) fn workspace(value: model::Workspace) -> v1::Workspace {
    v1::Workspace {
        id: value.id,
        display_name: value.display_name,
        created_at: Some(timestamp(value.created_at)),
        updated_at: Some(timestamp(value.updated_at)),
        resource_version: value.resource_version,
    }
}

pub(super) fn project(value: model::Project) -> v1::Project {
    v1::Project {
        id: value.id,
        display_name: value.display_name,
        created_at: Some(timestamp(value.created_at)),
        updated_at: Some(timestamp(value.updated_at)),
        resource_version: value.resource_version,
        workspace_id: value.workspace_id,
        goal: value.goal,
    }
}

pub(super) fn session(
    value: ayni_orchestrator_workflow_postgres::store::sessions::Session,
) -> v1::ConversationSession {
    v1::ConversationSession {
        id: value.id,
        workspace_id: value.workspace_id,
        project_id: value.project_id,
        conversation_id: value.conversation_id,
        generation: value.generation,
        state: value.state,
        idle_timeout_seconds: value.idle_timeout_seconds,
        can_replace: value.can_replace,
    }
}

pub(super) fn conversation(value: model::Conversation) -> v1::Conversation {
    v1::Conversation {
        id: value.id,
        workspace_id: value.workspace_id,
        project_id: value.project_id,
        title: value.title,
        created_at: Some(timestamp(value.created_at)),
        revision: value.revision,
    }
}

pub(super) fn receipt(value: model::CommandReceipt) -> v1::CommandReceipt {
    v1::CommandReceipt {
        command_id: value.command_id,
        kind: match value.kind {
            ReceiptKind::CreateWorkspace => v1::CommandReceiptKind::CreateWorkspace,
            ReceiptKind::CreateProject => v1::CommandReceiptKind::CreateProject,
            ReceiptKind::CreateConversation => v1::CommandReceiptKind::CreateConversation,
            ReceiptKind::SendMessage => v1::CommandReceiptKind::SendMessage,
        } as i32,
        workspace_id: value.workspace_id,
        project_id: value.project_id,
        conversation_id: value.conversation_id,
        message_id: value.message_id,
        turn_id: value.turn_id,
        accepted_at: Some(timestamp(value.accepted_at)),
        conversation_revision: value.conversation_revision,
        resume_cursor: value.resume_cursor,
        presentation_state: presentation(value.presentation_state),
    }
}

pub(super) fn snapshot(value: model::ConversationSnapshot) -> v1::ConversationSnapshot {
    v1::ConversationSnapshot {
        plans: value.plans.into_iter().map(plans::summary).collect(),
        workspace_id: value.workspace_id,
        project_id: value.project_id,
        conversation_id: value.conversation_id,
        conversation_revision: value.conversation_revision,
        recent_messages: value.recent_messages.into_iter().map(message).collect(),
        nonterminal_turns: value.nonterminal_turns.into_iter().map(turn).collect(),
        recent_terminal_turns: value
            .recent_terminal_turns
            .into_iter()
            .map(terminal)
            .collect(),
        history_counts: Some(v1::ConversationHistoryCounts {
            messages: value.message_count,
            turns: value.turn_count,
            has_older_history: value.has_older_history,
        }),
        history_snapshot_id: value.history_snapshot_id,
        current_activity: value.current_activity.map(activity),
        resume_cursor: value.resume_cursor,
        title: value.title,
        created_at: Some(timestamp(value.created_at)),
    }
}

pub(super) fn message(value: model::Message) -> v1::Message {
    v1::Message {
        id: value.id,
        conversation_id: value.conversation_id,
        turn_id: value.turn_id,
        role: match value.role {
            MessageRole::User => v1::MessageRole::User,
            MessageRole::Assistant => v1::MessageRole::Assistant,
        } as i32,
        content: value.content,
        accepted_at: Some(timestamp(value.accepted_at)),
        sequence: value.sequence,
    }
}

pub(super) fn turn(value: model::Turn) -> v1::Turn {
    v1::Turn {
        id: value.id,
        conversation_id: value.conversation_id,
        input_message_id: value.input_message_id,
        sequence: value.sequence,
        execution_binding_revision: value.execution_binding_revision,
        context_revision: value.context_revision,
        assignment_id: value.assignment_id,
        executor_generation: value.executor_generation,
        scheduling_state: match value.scheduling_state {
            SchedulingState::WaitingForPredecessor => v1::SchedulingState::WaitingForPredecessor,
            SchedulingState::Eligible => v1::SchedulingState::Eligible,
        } as i32,
        dispatch_state: match value.dispatch_state {
            DispatchState::NotEligible => v1::DispatchState::NotEligible,
            DispatchState::Pending => v1::DispatchState::Pending,
            DispatchState::Stored => v1::DispatchState::Stored,
            DispatchState::Transferred => v1::DispatchState::Transferred,
            DispatchState::Degraded => v1::DispatchState::Degraded,
        } as i32,
        execution_state: match value.execution_state {
            ExecutionState::NotStarted => v1::ExecutionState::NotStarted,
            ExecutionState::Processing => v1::ExecutionState::Processing,
            ExecutionState::Completed => v1::ExecutionState::Completed,
            ExecutionState::Failed => v1::ExecutionState::Failed,
        } as i32,
        response_message_id: value.response_message_id,
        failure_reason: value.failure_reason,
        accepted_at: Some(timestamp(value.accepted_at)),
        updated_at: Some(timestamp(value.updated_at)),
    }
}

pub(super) fn activity(value: model::Activity) -> v1::Activity {
    v1::Activity {
        id: value.id,
        conversation_id: value.conversation_id,
        conversation_revision: value.conversation_revision,
        cursor: value.cursor,
        turn_id: value.turn_id,
        stage: match value.stage.as_str() {
            "admission" => v1::ActivityStage::Admission,
            "scheduling" => v1::ActivityStage::Scheduling,
            "dispatch" => v1::ActivityStage::Dispatch,
            "execution" => v1::ActivityStage::Execution,
            "terminal" => v1::ActivityStage::Terminal,
            "provisioning" => v1::ActivityStage::Provisioning,
            "agent_wakeup" => v1::ActivityStage::AgentWakeup,
            "provider_request" => v1::ActivityStage::ProviderRequest,
            "provider_uncertain" => v1::ActivityStage::ProviderUncertain,
            "tool_execution" => v1::ActivityStage::ToolExecution,
            "preparation" => v1::ActivityStage::Preparation,
            _ => v1::ActivityStage::Unspecified,
        } as i32,
        presentation_state: presentation(value.presentation_state),
        waiting_on: waiting_on(&value.waiting_on),
        safe_reason: value.safe_reason,
        accepted_at: Some(timestamp(value.accepted_at)),
        safe_detail: value.safe_detail,
    }
}

pub(super) fn terminal(value: model::TerminalTurnSummary) -> v1::TerminalTurnSummary {
    let outcome = match value.presentation_state {
        PresentationState::Completed => Some(
            v1::terminal_turn_summary::Outcome::ResponseMessageId(value.response_message_id),
        ),
        PresentationState::Failed => Some(v1::terminal_turn_summary::Outcome::SafeFailureReason(
            value.safe_failure_reason,
        )),
        _ => None,
    };
    v1::TerminalTurnSummary {
        id: value.id,
        sequence: value.sequence,
        input_message_id: value.input_message_id,
        presentation_state: presentation(value.presentation_state),
        outcome,
        accepted_at: Some(timestamp(value.accepted_at)),
        terminal_at: Some(timestamp(value.terminal_at)),
    }
}

pub(super) fn history_record(value: model::HistoryRecord) -> v1::ConversationHistoryRecord {
    let role_or_state = match (value.kind, value.role_or_state.as_str()) {
        (HistoryKind::Message, "user") => v1::ConversationHistoryRoleOrState::UserMessage,
        (HistoryKind::Message, "assistant") => v1::ConversationHistoryRoleOrState::AssistantMessage,
        (HistoryKind::TerminalTurn, "completed") => {
            v1::ConversationHistoryRoleOrState::CompletedTurn
        }
        (HistoryKind::TerminalTurn, "failed") => v1::ConversationHistoryRoleOrState::FailedTurn,
        _ => v1::ConversationHistoryRoleOrState::Unspecified,
    };
    v1::ConversationHistoryRecord {
        sequence: value.sequence,
        id: value.id,
        kind: match value.kind {
            HistoryKind::Message => v1::ConversationHistoryRecordKind::Message,
            HistoryKind::TerminalTurn => v1::ConversationHistoryRecordKind::TerminalTurn,
        } as i32,
        role_or_state: role_or_state as i32,
        content_or_outcome: value.content_or_outcome,
        source_turn_id: value.source_turn_id,
        accepted_at: Some(timestamp(value.accepted_at)),
    }
}

const fn presentation(value: PresentationState) -> i32 {
    (match value {
        PresentationState::Accepted => v1::PresentationState::Accepted,
        PresentationState::Queued => v1::PresentationState::Queued,
        PresentationState::Processing => v1::PresentationState::Processing,
        PresentationState::Completed => v1::PresentationState::Completed,
        PresentationState::Failed => v1::PresentationState::Failed,
    }) as i32
}

pub(super) fn waiting_on(value: &str) -> i32 {
    (match value {
        "predecessor" => v1::WaitingOn::Predecessor,
        "delivery" => v1::WaitingOn::Delivery,
        "executor" => v1::WaitingOn::Executor,
        "provisioning" => v1::WaitingOn::Provisioning,
        _ => v1::WaitingOn::Unspecified,
    }) as i32
}

pub(super) const fn timestamp(value: model::Timestamp) -> Timestamp {
    Timestamp {
        seconds: value.seconds,
        nanos: value.nanos,
    }
}

pub(super) fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub(super) fn update(value: ConversationUpdate) -> Result<v1::ConversationUpdate, StoreError> {
    let kind = match (value.kind.as_str(), value.presentation_state) {
        ("task_execution_changed", _) => v1::ConversationUpdateKind::TaskExecutionChanged,
        ("task_execution_prepared", _) => v1::ConversationUpdateKind::TaskExecutionPrepared,
        ("plan_approved", _) => v1::ConversationUpdateKind::PlanApproved,
        ("plan_version_created", _) => v1::ConversationUpdateKind::PlanVersionCreated,
        ("conversation_created", _) => v1::ConversationUpdateKind::ConversationCreated,
        ("message_admitted", _) => v1::ConversationUpdateKind::MessageAdmitted,
        ("turn_state_changed", PresentationState::Completed | PresentationState::Failed) => {
            v1::ConversationUpdateKind::TerminalOutcomeAccepted
        }
        ("turn_state_changed", PresentationState::Queued) => {
            v1::ConversationUpdateKind::TurnSchedulingChanged
        }
        ("turn_state_changed", _) => v1::ConversationUpdateKind::TurnExecutionChanged,
        _ => return Err(StoreError::CorruptState),
    };
    Ok(v1::ConversationUpdate {
        update_id: value.update_id,
        activity_id: value.activity_id,
        cursor: value.cursor,
        conversation_revision: value.conversation_revision,
        kind: kind as i32,
        affected_resource_ids: value.affected_resource_ids,
        presentation_state: presentation(value.presentation_state),
        waiting_on: waiting_on(&value.waiting_on),
        safe_reason: value.safe_reason,
        accepted_at: Some(timestamp(value.accepted_at)),
    })
}

pub(super) fn agent_configuration(value: model::AgentConfiguration) -> v1::AgentConfiguration {
    v1::AgentConfiguration {
        connection_id: value.connection_id,
        workspace_id: value.workspace_id,
        revision_id: value.revision_id,
        instructions: value.instructions,
        provider: value.provider,
        model: value.model,
        max_input_tokens: value.max_input_tokens,
        max_output_tokens: value.max_output_tokens,
        max_total_tokens: value.max_total_tokens,
        idle_timeout_seconds: value.idle_timeout_seconds,
    }
}
