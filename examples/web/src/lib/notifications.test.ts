import { beforeEach, describe, expect, it, vi } from "vitest";
const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn(), warning: vi.fn(), info: vi.fn() }));
vi.mock("svelte-sonner", () => ({ toast }));
import { createNotificationEvents, notify, redirectNotification } from "./notifications.js";

describe("notification events", () => {
  beforeEach(() => vi.clearAllMocks());
  it("notifies once per action result but allows identical consecutive saves", () => {
    const events = createNotificationEvents();
    const result = { success: "Saved." };
    events.action(result);
    events.action(result);
    events.action({ success: "Saved." });
    expect(toast.success).toHaveBeenCalledTimes(2);
  });
  it("keeps safe action errors and request identifiers available until dismissed", () => {
    createNotificationEvents().action({ error: { message: "Permission denied.", requestId: "request-1" }, privateKey: "do-not-display" });
    expect(toast.error).toHaveBeenCalledWith("Action failed", { description: "Permission denied.\nRequest request-1", duration: Infinity });
  });
  it("deduplicates polling failures despite changing request identifiers, then notifies after recovery", () => {
    const events = createNotificationEvents();
    events.load("/repositories", { error: { message: "Unavailable", requestId: "1" } });
    events.load("/repositories", { error: { message: "Unavailable", requestId: "2" } });
    expect(toast.error).toHaveBeenCalledTimes(1);
    events.load("/repositories", {});
    events.load("/repositories", { error: { message: "Unavailable" } });
    expect(toast.error).toHaveBeenCalledTimes(2);
    events.load("/settings/ayni/certificates", { error: { message: "Unavailable" } });
    expect(toast.error).toHaveBeenCalledTimes(3);
  });
  it("handles auxiliary loader failures without toasting resource readiness or validation", () => {
    const events = createNotificationEvents();
    events.load("/repositories", { registryError: { message: "Registry unavailable" }, profile: { failure: { message: "Runtime needs repair" } } });
    events.action({ form: { errors: { displayName: ["Required"] } } });
    expect(toast.error).toHaveBeenCalledTimes(1);
    events.action({ form: { message: "Project could not be saved" } });
    expect(toast.error).toHaveBeenCalledTimes(2);
  });
  it("expires transient event notifications", () => {
    notify("success", "Copied");
    expect(toast.success).toHaveBeenCalledWith("Copied", { duration: 6000 });
  });
});


it("notifies completed redirect mutations without claiming failed actions succeeded", () => {
  vi.clearAllMocks();
  const action = new URL("https://example.test/repositories/repository:1?/remove");
  redirectNotification({ type: "failure", status: 409, data: {} }, action);
  expect(toast.success).not.toHaveBeenCalled();
  redirectNotification({ type: "redirect", status: 303, location: "/repositories" }, action);
  expect(toast.success).toHaveBeenCalledWith("Repository removed.", { duration: 6000 });
});

it("does not repeat an action notification when a refresh recreates its form object",()=>{
 vi.clearAllMocks();const events=createNotificationEvents();
 events.action({success:"Saved.",notificationId:"save:1"});events.action({success:"Saved.",notificationId:"save:1"});events.action(null);events.action({success:"Saved.",notificationId:"save:1"});
 expect(toast.success).toHaveBeenCalledTimes(1);
 events.action({success:"Saved.",notificationId:"save:2"});expect(toast.success).toHaveBeenCalledTimes(2);
});
