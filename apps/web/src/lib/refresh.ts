import { navigating } from "$app/state";
import { invalidate } from "$app/navigation";
import { createResourceWatch } from "./resource-watch.js";

export type RefreshScope = "app:conversations" | "app:workspaces" | "app:runtimes" | "app:tasks" | "app:connections" | "app:certificates" | "app:projects" | "app:repositories";
type Subscriber = { intervalMs: number; onResult?: ((ok: boolean) => void) | undefined };
type Entry = { subscribers: Set<Subscriber>; timer?: ReturnType<typeof setInterval> | undefined; pending?: Promise<void> | undefined; queued?: Promise<void> | undefined; stopStream?: (() => void) | undefined; streamHealthy?: boolean | undefined; dirty?: boolean | undefined };

/** One in-flight refresh and one timer per resource scope, shared by mounted consumers. */
export function createRefreshCoordinator(refresh: (scope: RefreshScope) => Promise<void>, canRefresh: () => boolean, watch?: (scope: RefreshScope, changed: () => void, status: (ok: boolean) => void) => () => void) {
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
      if(value.dirty && !value.queued && value.subscribers.size && canRefresh()){
        value.dirty=false;
        void request(scope).catch(()=>{for(const subscriber of value.subscribers)subscriber.onResult?.(false);});
      }
      if (!value.subscribers.size && !value.queued) entries.delete(scope);
    });
    return value.pending;
  }
  function event(scope: RefreshScope) {
    const value = entry(scope);
    value.dirty = true;
    if (!canRefresh() || value.pending || value.queued) return;
    value.dirty = false;
    void request(scope).catch(() => { for (const subscriber of value.subscribers) subscriber.onResult?.(false); });
  }
  function schedule(scope: RefreshScope, value: Entry) {
    clearInterval(value.timer);
    value.timer = undefined;
    if (!value.subscribers.size) { value.stopStream?.(); value.stopStream = undefined; if (!value.pending && !value.queued) entries.delete(scope); return; }
    const streaming = watch !== undefined && scope !== "app:tasks" && scope !== "app:workspaces" && scope !== "app:conversations";
    if (streaming && !value.stopStream) value.stopStream = watch(scope, () => event(scope), ok => {
      value.streamHealthy = ok;
      for (const subscriber of value.subscribers) subscriber.onResult?.(ok);
    });
    // A new consumer needs the existing transport state, not an optimistic
    // "live" label merely because another consumer already opened the stream.
    if (streaming && value.streamHealthy !== undefined) for (const subscriber of value.subscribers) subscriber.onResult?.(value.streamHealthy);
    const interval = Math.max(streaming ? 120_000 : 0, Math.min(...Array.from(value.subscribers, subscriber => subscriber.intervalMs)));
    let repairedAt = Date.now();
    value.timer = setInterval(() => {
      if (!canRefresh() || value.pending || (!value.dirty && Date.now() - repairedAt < interval)) return;
      value.dirty = false;
      repairedAt = Date.now();
      void request(scope).then(
        () => { for (const subscriber of value.subscribers) subscriber.onResult?.(!streaming || value.streamHealthy === true); },
        () => { for (const subscriber of value.subscribers) subscriber.onResult?.(false); },
      );
    }, 1_000);
  }
  function subscribe(scope: RefreshScope, subscriber: Subscriber) {
    if (!Number.isFinite(subscriber.intervalMs) || subscriber.intervalMs < 1_000) throw new Error("Refresh interval must be at least one second.");
    const value = entry(scope);
    value.subscribers.add(subscriber);
    schedule(scope, value);
    return () => { value.subscribers.delete(subscriber); schedule(scope, value); };
  }
  return { request, subscribe, event };
}

// Instantiated on the server too, but only mounted browser consumers subscribe.
// Avoid replacing resource versions or form defaults while an editor is active.
const resourceWatch = createResourceWatch(() => new EventSource("/updates/all"));
export function watchRefreshScope(scope: RefreshScope, changed: () => void, status: (ok: boolean) => void): () => void {
  if (scope === "app:tasks" || scope === "app:workspaces" || scope === "app:conversations") return () => {};
  return resourceWatch(scope, changed, status);
}

const coordinator = createRefreshCoordinator(invalidate, () =>
  typeof document !== "undefined" && document.visibilityState === "visible"
  && navigating.to === null
  && !document.querySelector("[data-refresh-paused]")
  && !document.querySelector('[role="dialog"][data-state="open"]')
  && !document.activeElement?.matches('input, textarea, select, [contenteditable="true"]'),
  watchRefreshScope);
export const refreshScope = coordinator.request;
export const subscribeRefresh = coordinator.subscribe;

export const resourceChanged = coordinator.event;
