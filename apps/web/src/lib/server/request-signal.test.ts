import { describe, expect, it } from "vitest";
import { browserRequestSignal } from "./request-signal.js";

const signalStoreKey = Symbol.for("ayni.orchestrator.web.request-signals");

describe("browser response lifetime signal", () => {
  it("combines the adapter request and private response signals", () => {
    const responseCancellation = new AbortController();
    (globalThis as Record<symbol, unknown>)[signalStoreKey] = new Map([
      ["internal-test", responseCancellation.signal],
    ]);
    const signal = browserRequestSignal(new Request("http://127.0.0.1/", {
      headers: { "x-ayni-internal-request-signal": "internal-test" },
    }));
    expect(signal.aborted).toBe(false);
    responseCancellation.abort();
    expect(signal.aborted).toBe(true);
  });

  it("falls back to the adapter request signal outside the custom runtime", () => {
    const request = new Request("http://127.0.0.1/");
    expect(browserRequestSignal(request)).toBe(request.signal);
  });
});
