import { describe, expect, it, vi } from "vitest";
import { createResourceWatch } from "./resource-watch.js";

function fixture() {
  const events = new EventTarget();
  const close = vi.fn();
  const source = Object.assign(events, { onopen: undefined as (() => void) | undefined, close });
  const open = vi.fn(() => source as unknown as EventSource);
  return { source, open, close, watch: createResourceWatch(open) };
}

describe("shared browser resource watch", () => {
  it("uses one connection for all five scopes and releases it after the last subscriber", () => {
    const { watch, open, close } = fixture();
    const scopes = ["app:connections", "app:certificates", "app:projects", "app:runtimes", "app:repositories"] as const;
    const stops = scopes.map(scope => watch(scope, vi.fn(), vi.fn()));
    expect(open).toHaveBeenCalledTimes(1);
    for (const stop of stops.slice(0, -1)) stop();
    expect(close).not.toHaveBeenCalled();
    stops.at(-1)?.();
    expect(close).toHaveBeenCalledTimes(1);
    const stop = watch("app:projects", vi.fn(), vi.fn());
    expect(open).toHaveBeenCalledTimes(2);
    stop();
  });

  it("routes scoped changes, resynchronizes late subscribers, and reports transport status", () => {
    const { watch, source } = fixture();
    const connections = vi.fn(), projects = vi.fn(), status = vi.fn();
    const stopConnections = watch("app:connections", connections, status);
    source.onopen?.();
    expect(status).toHaveBeenLastCalledWith(true);
    const stopProjects = watch("app:projects", projects, status);
    expect(projects).toHaveBeenCalledTimes(1);
    source.dispatchEvent(new MessageEvent("changed", { data: '{"scope":"connections"}' }));
    expect(connections).toHaveBeenCalledTimes(1);
    expect(projects).toHaveBeenCalledTimes(1);
    source.dispatchEvent(new Event("error"));
    expect(status).toHaveBeenLastCalledWith(false);
    source.onopen?.();
    expect(status).toHaveBeenLastCalledWith(true);
    stopConnections(); stopProjects();
  });
});
