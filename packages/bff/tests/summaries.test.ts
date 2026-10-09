import { describe, expect, it } from "vitest";
import { mapSummaryReadiness } from "../src/summaries.js";
import { BffMappingError } from "../src/mapping.js";
describe("summary readiness mapping", () => {
  it("retains an explicit unavailable result", () => {
    expect(mapSummaryReadiness({complete:false,ready:false,operationalProfiles:0,readyProfiles:0})).toMatchObject({complete:false,ready:false});
  });
  it("rejects missing and contradictory readiness", () => {
    for(const value of [undefined,{complete:false,ready:true,operationalProfiles:1,readyProfiles:1},{complete:true,ready:true,operationalProfiles:0,readyProfiles:0},{complete:true,ready:false,operationalProfiles:1,readyProfiles:2}]) {
      expect(()=>mapSummaryReadiness(value)).toThrow(BffMappingError);
    }
  });
  it("does not declare a multi-repository project ready from aggregate counts", () => {
    expect(mapSummaryReadiness({complete:true,ready:false,operationalProfiles:1,readyProfiles:1})).toMatchObject({ready:false});
  });
});
