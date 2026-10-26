import { describe, it, expect } from "vitest";
import { beginManifest, consumeManifest } from "./github-manifest.js";
describe("GitHub manifest one-time state",()=>{
 it("consumes atomically and rejects replay",()=>{const state=beginManifest();expect(consumeManifest(state)).toBe(true);expect(consumeManifest(state)).toBe(false);});
 it("rejects unknown states",()=>{expect(consumeManifest("unknown")).toBe(false);});
});
