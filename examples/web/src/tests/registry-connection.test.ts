import {describe,expect,it} from "vitest";
import {registryConfiguration,registryCredentialStatus,registryFailureMessage} from "$lib/connection.js";
const verified={kind:"ghcr",enabled:true,status:"ready",failureReason:"",testPush:"passed",testPull:"passed"};
describe("current registry availability and retained probe results",()=>{
  it("does not present old passing checks as current access after a Secret lookup failure",()=>{
    const unavailable={...verified,status:"failed",failureReason:"connection_secret_unavailable"};
    expect(registryCredentialStatus(unavailable)).toBe("Unavailable");
    expect(registryConfiguration([unavailable])).toMatchObject({status:"warning",description:expect.stringContaining("historical")});
    expect(registryFailureMessage(unavailable)).toContain("recheck automatically");
  });
  it("keeps old results unverified while changed credentials are being checked",()=>{
    const pending={...verified,status:"pending"};
    expect(registryCredentialStatus(pending)).toBe("Checking");
    expect(registryConfiguration([pending])).toMatchObject({status:"warning",description:expect.stringContaining("pending")});
  });
  it("reports available active credentials independently of a failed replacement",()=>{
    const active={...verified,failureReason:"connection_secret_unavailable"};
    expect(registryCredentialStatus(active)).toBe("Available");
    expect(registryConfiguration([active]).status).toBe("ready");
    expect(registryFailureMessage(active)).toBe("connection_secret_unavailable");
  });
  it("does not let one unavailable connection hide another verified registry",()=>{
    expect(registryConfiguration([{...verified,status:"failed",failureReason:"connection_secret_unavailable"},verified]).status).toBe("ready");
    expect(registryCredentialStatus({...verified,enabled:false})).toBe("Disabled");
  });
});
