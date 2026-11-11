export type LogSource = { id: string; observedAtUnixMs?: string; namespace?: string; podName?: string; podUid?: string; container?: string; restart?: number; component?: string; executionId?: string; stage?: string; provenance?: string; captureStatus?: string };
export type LogRecord = { source?: LogSource; sequence?: string; event?: string; stream?: string; data?: string; actionId?: string; observedAtUnixMs?: string; lostBytes?: string; exitCode?: number; signal?: number };
export type LogEntry = { cursor: string; record?: LogRecord };
export type LogPage = { entries?: LogEntry[]; nextCursor?: string; expiredThroughCursor?: string };

// Never interpret terminal escapes, HTML or directional formatting in raw output.
export function displayLog(encoded = ''): string {
  try {
    const bytes = Uint8Array.from(atob(encoded), character => character.charCodeAt(0));
    return new TextDecoder().decode(bytes).replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/g, character => `\\u${character.charCodeAt(0).toString(16).padStart(4, '0')}`);
  } catch { return '[Output could not be decoded]'; }
}

export function appendLogPage(current: LogEntry[], page: LogPage): LogEntry[] {
  const ids = new Set(current.map(entry => entry.cursor));
  return [...current, ...(page.entries ?? []).filter(entry => !ids.has(entry.cursor))].slice(-1000);
}

export function decodeLogEntries(entries: LogEntry[]): Map<string, string> {
  const decoders = new Map<string, TextDecoder>();
  const text = new Map<string, string>();
  const last = new Map<string, string>();
  for (const entry of entries) {
    const record = entry.record;
    if (!record?.data) continue;
    const identity = `${record.source?.id}:${record.actionId}:${record.stream}`;
    const decoder = decoders.get(identity) ?? new TextDecoder();
    decoders.set(identity, decoder);
    last.set(identity, entry.cursor);
    try {
      const bytes = Uint8Array.from(atob(record.data), c => c.charCodeAt(0));
      text.set(entry.cursor, decoder.decode(bytes, {stream:true}).replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/g, c => `\\u${c.charCodeAt(0).toString(16).padStart(4,'0')}`));
    } catch { text.set(entry.cursor, '[Output could not be decoded]'); }
  }
  for (const [identity, decoder] of decoders) {
    const cursor = last.get(identity)!;
    text.set(cursor, (text.get(cursor) ?? "") + decoder.decode());
  }
  return text;
}
