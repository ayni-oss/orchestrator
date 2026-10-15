import {test} from "node:test";
import assert from "node:assert/strict";
import {once} from "node:events";
import {plannerStub} from "./planner-stub.mjs";
test("planner stub emits Responses wire format and exposes deterministic hold/release without credentials",async()=>{
  const server=plannerStub();server.listen(0,"127.0.0.1");await once(server,"listening");
  const base=`http://127.0.0.1:${server.address().port}`;
  try{
    const models=await fetch(`${base}/v1/models`).then(r=>r.json());assert(models.data.some(m=>m.id==="gpt-5.6-sol"));
    await fetch(`${base}/fixture/hold`,{method:"POST"});
    const response=fetch(`${base}/v1/responses`,{method:"POST",headers:{authorization:"Bearer synthetic-private-key"},body:JSON.stringify({model:"gpt-5.6-sol",input:"Synthetic prompt"})});
    let stats;for(let i=0;i<50;i++){stats=await fetch(`${base}/fixture/stats`).then(r=>r.json());if(stats.pending===1)break;await new Promise(resolve=>setTimeout(resolve,10));}
    assert.deepEqual(stats,{calls:1,completed:0,pending:1,holding:true});assert(!JSON.stringify(stats).includes("synthetic-private-key"));
    await fetch(`${base}/fixture/release`,{method:"POST"});
    const body=await response.then(r=>r.json());assert.equal(body.status,"completed");assert.equal(body.output[0].content[0].type,"output_text");assert.equal(body.usage.total_tokens,24);
    assert.equal((await fetch(`${base}/unsupported`)).status,404);
  }finally{server.closeAllConnections();server.close();}
});

test("planner stub starts through the symlink used by ConfigMap volumes",async()=>{
  const {mkdtemp,symlink,rm}=await import("node:fs/promises");
  const {tmpdir}=await import("node:os");
  const {join}=await import("node:path");
  const {fileURLToPath}=await import("node:url");
  const {spawn}=await import("node:child_process");
  const reserve=plannerStub();reserve.listen(0,"127.0.0.1");await once(reserve,"listening");const port=reserve.address().port;await new Promise(resolve=>reserve.close(resolve));
  const directory=await mkdtemp(join(tmpdir(),"planner-stub-link-")),link=join(directory,"server.mjs");
  await symlink(fileURLToPath(new URL("./planner-stub.mjs",import.meta.url)),link);
  const child=spawn(process.execPath,[link],{env:{...process.env,PORT:String(port)},stdio:"ignore"});
  const closed=once(child,"exit");
  try{
    let ready=false;for(let i=0;i<100;i++){
      try{ready=(await fetch(`http://127.0.0.1:${port}/healthz`)).ok;}catch{/* Wait for process startup. */}
      if(ready)break;await new Promise(resolve=>setTimeout(resolve,10));
    }
    assert(ready,"symlink entry point should start its HTTP listener");
  }finally{child.kill("SIGTERM");await closed;await rm(directory,{recursive:true,force:true});}
});
