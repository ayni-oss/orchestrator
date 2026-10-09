import { invalidate } from "$app/navigation";

export type RefreshScope = "app:runtimes" | "app:tasks" | "app:connections" | "app:certificates";
type Subscriber = { intervalMs: number; onResult?: ((ok: boolean) => void) | undefined };
type Entry = { subscribers: Set<Subscriber>; timer?: ReturnType<typeof setInterval> | undefined; pending?: Promise<void> | undefined; queued?: Promise<void> | undefined };

/** One in-flight refresh and one timer per resource scope, shared by mounted consumers. */
export function createRefreshCoordinator(refresh: (scope: RefreshScope) => Promise<void>, canRefresh: () => boolean) {
  const entries = new Map<RefreshScope, Entry>();
  function entry(scope: RefreshScope) {
    let value = entries.get(scope);
    if (!value) { value = { subscribers: new Set() }; entries.set(scope, value); }
    return value;
  }
  function request(scope: RefreshScope, force = false): Promise<void> {
    if (!force && !canRefresh()) return Promise.resolve();
    const value = entry(scope);
    if (value.queued) return value.queued;
    if (value.pending) {
      if (!force) return value.pending;
      // A read started before a save cannot satisfy its post-write refresh.
      // Coalesce concurrent saves into one trailing read, even if the old read failed.
      value.queued = value.pending.catch(() => undefined).then(() => {
        value.queued = undefined;
        return request(scope, true);
      });
      return value.queued;
    }
    value.pending = Promise.resolve().then(() => refresh(scope)).finally(() => {
      value.pending = undefined;
      if (!value.subscribers.size && !value.queued) entries.delete(scope);
    });
    return value.pending;
  }
  function schedule(scope: RefreshScope, value: Entry) {
    clearInterval(value.timer);
    value.timer = undefined;
    if (!value.subscribers.size) { if (!value.pending && !value.queued) entries.delete(scope); return; }
    const interval = Math.min(...Array.from(value.subscribers, subscriber => subscriber.intervalMs));
    value.timer = setInterval(() => {
      if (!canRefresh() || value.pending) return;
      void request(scope).then(
        () => { for (const subscriber of value.subscribers) subscriber.onResult?.(true); },
        () => { for (const subscriber of value.subscribers) subscriber.onResult?.(false); },
      );
    }, interval);
  }
  function subscribe(scope: RefreshScope, subscriber: Subscriber) {
    if (!Number.isFinite(subscriber.intervalMs) || subscriber.intervalMs < 1_000) throw new Error("Refresh interval must be at least one second.");
    const value = entry(scope);
    value.subscribers.add(subscriber);
    schedule(scope, value);
    return () => { value.subscribers.delete(subscriber); schedule(scope, value); };
  }
  return { request, subscribe };
}

// Instantiated on the server too, but only mounted browser consumers subscribe.
// Avoid replacing resource versions or form defaults while an editor is active.
const coordinator = createRefreshCoordinator(invalidate, () =>
  typeof document !== "undefined" && document.visibilityState === "visible"
  && !document.querySelector('[role="dialog"][data-state="open"]')
  && !document.activeElement?.matches('input, textarea, select, [contenteditable="true"]'),
);
export const refreshScope = coordinator.request;
export const subscribeRefresh = coordinator.subscribe;
