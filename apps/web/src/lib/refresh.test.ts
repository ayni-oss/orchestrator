import { afterEach, describe, expect, it, vi } from "vitest";
vi.mock("$app/navigation", () => ({invalidate:vi.fn()}));
import { createRefreshCoordinator } from "./refresh.js";

afterEach(() => { vi.useRealTimers(); });
describe("scoped resource refresh", () => {
  it("shares one poll between consumers and stops after the last unmount", async () => {
    vi.useFakeTimers();
    const invalidate=vi.fn().mockResolvedValue(undefined);
    const refresh=createRefreshCoordinator(invalidate,()=>true);
    const first=refresh.subscribe("app:runtimes",{intervalMs:10_000});
    const second=refresh.subscribe("app:runtimes",{intervalMs:30_000});
    await vi.advanceTimersByTimeAsync(10_000);
    expect(invalidate).toHaveBeenCalledExactlyOnceWith("app:runtimes");
    first();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(invalidate).toHaveBeenCalledTimes(2);
    second();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(invalidate).toHaveBeenCalledTimes(2);
  });
  it("coalesces events and polling while an invalidation is in flight", async () => {
    vi.useFakeTimers();
    let finish!:()=>void;
    const invalidate=vi.fn(()=>new Promise<void>(resolve=>{finish=resolve;}));
    const refresh=createRefreshCoordinator(invalidate,()=>true);
    const stop=refresh.subscribe("app:tasks",{intervalMs:10_000});
    const pending=refresh.request("app:tasks");
    expect(refresh.request("app:tasks")).toBe(pending);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(invalidate).toHaveBeenCalledTimes(1);
    finish();await pending;stop();
  });
  it("follows an in-flight pre-save read with one forced read after the mutation", async () => {
    let finish!:()=>void;
    const invalidate=vi.fn().mockImplementationOnce(()=>new Promise<void>(resolve=>{finish=resolve;})).mockResolvedValue(undefined);
    const refresh=createRefreshCoordinator(invalidate,()=>true);
    const beforeSave=refresh.request("app:connections");
    await Promise.resolve();
    const afterSave=refresh.request("app:connections",true);
    expect(refresh.request("app:connections",true)).toBe(afterSave);
    expect(afterSave).not.toBe(beforeSave);
    expect(invalidate).toHaveBeenCalledTimes(1);
    finish();
    await afterSave;
    expect(invalidate).toHaveBeenCalledTimes(2);
  });
  it("recovers a terminal event received while editing through the next shared poll", async () => {
    vi.useFakeTimers();
    let editing=true;
    const invalidate=vi.fn().mockResolvedValue(undefined);
    const refresh=createRefreshCoordinator(invalidate,()=>!editing);
    const stop=refresh.subscribe("app:tasks",{intervalMs:10_000});
    await refresh.request("app:tasks");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(invalidate).not.toHaveBeenCalled();
    editing=false;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(invalidate).toHaveBeenCalledExactlyOnceWith("app:tasks");
    stop();
  });
  it("pauses background refresh while hidden or editing, allows intentional mutation refresh, and recovers from errors", async () => {
    vi.useFakeTimers();
    let allowed=false;
    const invalidate=vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValue(undefined);
    const onResult=vi.fn();
    const refresh=createRefreshCoordinator(invalidate,()=>allowed);
    const stop=refresh.subscribe("app:certificates",{intervalMs:10_000,onResult});
    await vi.advanceTimersByTimeAsync(30_000);
    expect(invalidate).not.toHaveBeenCalled();
    await expect(refresh.request("app:certificates",true)).rejects.toThrow("offline");
    allowed=true;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(onResult).toHaveBeenCalledWith(true);
    stop();
  });
});
