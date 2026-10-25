// The browser only uses the gateway's existing allowlisted ProtoJSON methods.
export async function request(method, body) {
  const payload = JSON.stringify(body);
  if (new TextEncoder().encode(payload).length > 64 * 1024) {
    throw Object.assign(new Error('This command exceeds the gateway size limit. Shorten the text and try again.'), { code: 'InvalidArgument' });
  }
  const response = await fetch(`/api/${method}`, {
    method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' },
    body: payload, signal: AbortSignal.timeout(15000),
  });
  if (response.status === 413) {
    throw Object.assign(new Error('The gateway rejected this command before admission because it is too large.'), { code: 'InvalidArgument' });
  }
  let result;
  try { result = await response.json(); }
  catch { throw new Error('The gateway response could not be read. Keep the original command for an exact retry.'); }
  if (!response.ok) {
    const error = new Error(`${result.reason || result.code || 'Request failed'}${result.requestId ? ` · ${result.requestId}` : ''}`);
    Object.assign(error, { code: result.code, reason: result.reason });
    throw error;
  }
  return result;
}
export function readSaved(storage, key, fallback) {
  try { return JSON.parse(storage.getItem(key)) ?? fallback; } catch { return fallback; }
}
export function save(storage, key, value) {
  try {
    if (value == null) storage.removeItem(key);
    else storage.setItem(key, JSON.stringify(value));
    return true;
  } catch { return false; }
}
export function consoleLink(scope) {
  const query = new URLSearchParams(Object.entries(scope).filter(([, value]) => value));
  return `/?${query}`;
}
export function readable(value = '') {
  return value.replace(/^(PRESENTATION_STATE|WAITING_ON|ACTIVITY_STAGE|EXECUTION_STATE|DISPATCH_STATE)_/, '').toLowerCase().replaceAll('_', ' ');
}
export function count(value) {
  return value == null ? 'Unknown' : BigInt(value).toLocaleString();
}
export function elapsed(start, end = Date.now()) {
  if (!start) return 'Not recorded';
  const seconds = Math.max(0, Math.floor((Number(end) - Date.parse(start)) / 1000));
  if (!Number.isFinite(seconds)) return 'Not recorded';
  return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}
