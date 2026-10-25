// A frozen history boundary keeps pagination stable while live activity changes.
export const emptyTimeline = () => ({ entries: [], snapshotRevision: '0', nextBeforeRevision: '0', retainedFromRevision: '0', busy: false, error: '' });
export function mergeTimeline(previous, page, append) {
  const entries = append ? [...previous.entries, ...(page.entries || [])] : page.entries || [];
  const unique = new Map(entries.map(entry => [entry.activity.id, entry]));
  return {
    entries: [...unique.values()], snapshotRevision: page.snapshotRevision || '0',
    nextBeforeRevision: page.nextBeforeRevision || '0', retainedFromRevision: page.retainedFromRevision || '0',
    busy: false, error: '',
  };
}
export function hasNewActivity(snapshot, timeline) {
  return timeline.snapshotRevision !== '0' && BigInt(snapshot?.conversationRevision || '0') > BigInt(timeline.snapshotRevision);
}
