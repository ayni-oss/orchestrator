import { describe,it,expect } from "vitest";
import { safeGithubLink,safeText,safeReconciliation,mapInspection } from "../src/onboarding.js";
import { InspectAyniRepositoryResponse } from "@ayni-orchestrator/grpc-bindings/server";

describe("operator onboarding boundary",()=>{
 it("drops credential-bearing and non-provider links",()=>{
  for(const url of ["https://secret@github.com/a/b","javascript:alert(1)","https://github.com.evil/a","https://github.com:8443/a"]){expect(safeGithubLink(url)).toBeUndefined();}
  expect(safeGithubLink("https://github.com/ayni-oss/sandbox/pull/8")).toBe("https://github.com/ayni-oss/sandbox/pull/8");
 });
 it("exposes only bounded reconciliation metadata",()=>{
  const result=safeReconciliation([{key:"signing-secret",value:"private-value"},{key:"build-job",value:"private-job-name"},{key:"reason-code",value:"a\n".repeat(1000)}]);
  expect(result).toHaveLength(1);
  expect(result[0]?.value.length).toBe(512);
  expect(JSON.stringify(result)).not.toContain("private");
  expect(safeText("a\u0000b")).toBe("a b");
 });
 it("keeps safe retry classification and history in the operator projection",()=>{
  const metadata=[{key:"retry-eligibility",value:"operator_required"},{key:"error-class",value:"reporting"},{key:"last-error",value:"runtime_report_missing"},{key:"failed-executions",value:"2"},{key:"legacy-failures",value:"3"}];
  expect(safeReconciliation(metadata)).toEqual(metadata);
 });
 it("reports trust-policy observation separately from compatibility",()=>{
  const inspection=mapInspection(InspectAyniRepositoryResponse.fromPartial({lockCompatible:true,trustPolicyPinned:false,reason:"trust_policy_missing"}));
  expect(inspection.lockCompatible).toBe(true);
  expect(inspection.trustPinned).toBe(false);
  expect(inspection.reason).toBe("trust_policy_missing");
 });
});
