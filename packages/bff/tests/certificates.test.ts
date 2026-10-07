import { describe, expect, it } from "vitest";
import { Certificate } from "@ayni-orchestrator/grpc-bindings/server";
import { mapCertificate } from "../src/certificates.js";
import { safeReconciliation } from "../src/onboarding.js";
describe("certificate browser metadata",()=>{
  it("allowlists public metadata and excludes private seed and execution history",()=>{
    const wire={...Certificate.fromPartial({id:"certificate:1",displayName:"Team",publicKey:"b".repeat(64),revision:"2",usageCount:"3",retiredSecretRevisions:[{secretName:"retired",secretUid:"uid"}]}),signingSeed:"a".repeat(64),credentials:{seed:"hidden"}};
    const result=mapCertificate(wire);
    expect(result.publicKey).toBe("b".repeat(64));expect(result.usageCount).toBe("3");
    expect(JSON.stringify(result)).not.toContain("hidden");expect(result).not.toHaveProperty("signingSeed");expect(result).not.toHaveProperty("retiredSecretRevisions");
  });
  it("retains trusted public pull request links and rejects unsafe links",()=>{
    expect(safeReconciliation([{key:"certificate-trust-pr",value:"https://github.com/example/repo/pull/1"},{key:"certificate-trust-state",value:"open"},{key:"signing_seed",value:"private"},{key:"certificate-trust-pr",value:"javascript:alert(1)"}])).toEqual([{key:"certificate-trust-pr",value:"https://github.com/example/repo/pull/1"},{key:"certificate-trust-state",value:"open"}]);
  });
});
