// Keep protocol identities as decimal strings, including values above 2^53.
export function newerOrEqual(next, current = '0') {
  return BigInt(next || '0') >= BigInt(current || '0');
}
export function describe(value = '') {
  return value.replace(/^(PRESENTATION_STATE|WAITING_ON|ACTIVITY_STAGE)_/, '').toLowerCase().replaceAll('_', ' ');
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
