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
  return {
    binding: `${call.provider || 'Unknown provider'} / ${call.model || 'Unknown model'}`,
    state: describe(call.state),
    usage: `Input ${tokenCount(call.inputTokens)} · output ${tokenCount(call.outputTokens)} · total ${tokenCount(call.totalTokens)}`,
    reservation: `Input ${tokenCount(call.reservedInputTokens)} · output ${tokenCount(call.reservedOutputTokens)} · total ${tokenCount(call.reservedTotalTokens)}`,
    notice: call.state === 'uncertain'
      ? 'The external outcome is unknown. This call will not be retried automatically. Retain its call ID for reconciliation.'
      : call.cancellationRequested && unresolved
        ? 'Cancellation requested. The external outcome is unconfirmed; this does not confirm remote revocation or a refund.'
        : call.cancellationRequested
          ? 'Cancellation requested. Retained provider result and usage remain part of the record.'
          : '',
  };
}
