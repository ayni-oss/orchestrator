// Keep protocol identities as decimal strings, including values above 2^53.
export function newerOrEqual(next, current = '0') {
  return BigInt(next || '0') >= BigInt(current || '0');
}
export function describe(value = '') {
  return value.replace(/^(PRESENTATION_STATE|WAITING_ON|ACTIVITY_STAGE|EXECUTION_STATE)_/, '').toLowerCase().replaceAll('_', ' ');
}
export function scopeKey(scope) {
  return [scope.workspaceId, scope.projectId, scope.conversationId].join('/');
}
export function makeCommand(method, body, id = crypto.randomUUID()) {
  return { method, body: { ...body, idempotencyKey: id } };
}
export function appendActivity(existing, update) {
  if (existing.some(item => item.cursor === update.cursor)) return existing;
  return [...existing, update].slice(-100);
}

// ProtoJSON optional usage preserves absence separately from an observed zero.
export function tokenCount(value) {
  if (value == null || value === '') return 'Unknown';
  if (!/^[0-9]+$/.test(String(value))) return 'Unknown';
  return BigInt(value).toLocaleString('en-US');
}
export function providerCallView(call) {
  const unresolved = ['prepared', 'sending', 'uncertain'].includes(call.state);
  const uncertainTools = (call.toolInvocations || []).filter(tool => /^mcp_[a-z0-9_]{1,60}$/.test(tool.name) && ['sending', 'uncertain'].includes(tool.state));
  return {
    binding: `${call.provider || 'Unknown provider'} / ${call.model || 'Unknown model'}`,
    state: describe(call.state),
    usage: `Input ${tokenCount(call.inputTokens)} · output ${tokenCount(call.outputTokens)} · total ${tokenCount(call.totalTokens)}`,
    tools: (call.toolInvocations || []).filter(tool => (['read_workspace_instructions', 'read_project_status', 'read_conversation_history'].includes(tool.name) || /^mcp_[a-z0-9_]{1,60}$/.test(tool.name))).map(tool => `${tool.name} · ${tool.state || 'retained'}`),
    toolNotice: uncertainTools.length ? `The result of an external tool action is unknown. Check the external system before repeating it. ${uncertainTools.map(tool => `${tool.name} · invocation ${/^[a-zA-Z0-9_-]{1,128}$/.test(tool.invocationId || '') ? tool.invocationId : 'unavailable'}`).join('; ')}` : '',
    reservation: `Input ${tokenCount(call.reservedInputTokens)} · output ${tokenCount(call.reservedOutputTokens)} · total ${tokenCount(call.reservedTotalTokens)}`,
    notice: call.uncertaintyAcknowledged
      ? 'Uncertain outcome acknowledged. The call may have incurred a charge. Its evidence remains retained and this call will not be retried.'
      : call.state === 'uncertain'
      ? 'The external outcome is unknown. This call will not be retried automatically. Retain its call ID for reconciliation.'
      : call.cancellationRequested && unresolved
        ? 'Cancellation requested. The external outcome is unconfirmed; this does not confirm remote revocation or a refund.'
        : call.cancellationRequested
          ? 'Cancellation requested. Retained provider result and usage remain part of the record.'
          : '',
  };
}

// Freeze the exact origin and reviewed configuration before a retryable command.
export function outcomeAssessmentCommand(scope, configuration, count, consent) {
  const maxAssessments = Number(count);
  if (!scope?.conversationId || !scope.projectId || !scope.workspaceId || configuration?.workspaceId !== scope.workspaceId || !configuration.revisionId) throw new Error('Choose a conversation and load its workspace configuration.');
  if (!Number.isInteger(maxAssessments) || maxAssessments < 0 || maxAssessments > 10) throw new Error('Choose an assessment allowance from 0 to 10.');
  if (maxAssessments > 0 && !consent) throw new Error('Confirm the assessment allowance and model limits before saving.');
  return { workspaceId:scope.workspaceId, projectId:scope.projectId, conversationId:scope.conversationId, expectedConfigurationRevisionId:configuration.revisionId, maxAssessments };
}
