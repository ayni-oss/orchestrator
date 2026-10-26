import { describe, expect, it } from "vitest";
import { acceptedRequestId } from "./request-id.js";

describe("request correlation", () => {
  it("retains only bounded safe request IDs", () => {
    expect(acceptedRequestId("browser.request-1")).toBe("browser.request-1");
    expect(acceptedRequestId("Bearer [REDACTED] value")).toMatch(/^[0-9a-f-]{36}$/u);
    expect(acceptedRequestId("a".repeat(65))).toMatch(/^[0-9a-f-]{36}$/u);
  });
});
