export const historyBatchSize = 10;

export type HistoryPage<T> = { items: T[]; next: string };
export type ReadHistoryPage<T> = (afterId: string) => Promise<HistoryPage<T>>;

// The runtime workflow uses the 20 most recent records independently of the page on screen.
export async function readRecentHistory<T>(read: ReadHistoryPage<T>): Promise<HistoryPage<T>> {
 const first = await read('');
 if (!first.next) return first;
 const second = await read(first.next);
 return {items:[...first.items, ...second.items], next:second.next};
}
