type Scope = "app:connections" | "app:certificates" | "app:projects" | "app:runtimes" | "app:repositories";
type Subscriber = { changed: () => void; status: (ok: boolean) => void };

/** Share one browser connection; leave HTTP/1.1 slots for navigation and tasks. */
export function createResourceWatch(open: () => EventSource) {
  const subscribers = new Map<Scope, Set<Subscriber>>();
  let source: EventSource | undefined;
  let live = false;
  const status = (ok: boolean) => {
    live = ok;
    for (const values of subscribers.values()) for (const value of values) value.status(ok);
  };
  return (scope: Scope, changed: () => void, onStatus: (ok: boolean) => void): (() => void) => {
    const subscriber = { changed, status: onStatus };
    let values = subscribers.get(scope);
    if (!values) { values = new Set(); subscribers.set(scope, values); }
    values.add(subscriber);
    if (!source) {
      source = open();
      source.onopen = () => status(true);
      source.addEventListener("error", () => status(false));
      source.addEventListener("changed", event => {
        try {
          const update = JSON.parse((event as MessageEvent<string>).data) as { scope?: string };
          for (const value of subscribers.get(`app:${update.scope}` as Scope) ?? []) value.changed();
        } catch { status(false); }
      });
    } else {
      onStatus(live);
      // This scope may have missed its initial snapshot before it subscribed.
      changed();
    }
    return () => {
      values.delete(subscriber);
      if (!values.size) subscribers.delete(scope);
      if (!subscribers.size) { source?.close(); source = undefined; live = false; }
    };
  };
}
