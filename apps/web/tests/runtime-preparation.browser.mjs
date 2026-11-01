import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createServer as createSocket} from 'node:net';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {randomUUID} from 'node:crypto';
import {setTimeout as delay} from 'node:timers/promises';
import {chromium} from 'playwright';
import {exerciseRuntimePreparation,exerciseDefaultBranchRecheck,exerciseRepositoryOnboarding} from './runtime-preparation-ui.mjs';

// This fixture qualifies UI command/recovery behavior only, never runtime readiness.
test('runtime preparation UI retains explicit grants and recovers exact commands', {timeout:120000}, async()=>{
 const repositoryId=randomUUID();
 const repository={id:repositoryId,connectionId:randomUUID(),installationId:'123',providerRepositoryId:'456',fullName:'fixture/runtime-ui',defaultBranch:'main',private:true,connectedAtUnixMs:String(Date.now())};
 const gateway=createServer((request,response)=>{
  const body=request.url==='/api/ListRepositories'?{repositories:[repository]}:request.url==='/api/ListWorkspaces'?{workspaces:[]}:request.url==='/api/ListConnections'?{connections:[{id:repository.connectionId,displayName:'Fixture GitHub',capability:'source_control',enabled:true,activeCredentialRevision:'credential',revision:'1'}]}:{};
  response.writeHead(200,{'Content-Type':'application/json'});response.end(JSON.stringify(body));
 });
 let child,browser;
 try {
  gateway.listen(0,'127.0.0.1');await once(gateway,'listening');
  const socket=createSocket();socket.listen(0,'127.0.0.1');await once(socket,'listening');const port=socket.address().port;await new Promise(resolve=>socket.close(resolve));
  const origin=`http://127.0.0.1:${port}`;
  const env={...process.env,AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT:String(gateway.address().port),HOST:'127.0.0.1',PORT:String(port)};
  child=process.env.AYNI_PRODUCT_PRODUCTION==='1'?spawn('node',['apps/web/build/index.js'],{env,stdio:'ignore',detached:true}):spawn('pnpm',['--dir','apps/web','exec','vite','dev','--host','127.0.0.1','--port',String(port),'--strictPort'],{env,stdio:'ignore',detached:true});
  let ready=false;for(let i=0;i<120;i++){try{if((await fetch(origin)).ok){ready=true;break;}}catch{}await delay(250);}assert.ok(ready,'UI fixture server did not start');
  browser=await chromium.launch({headless:true});const page=await browser.newPage();
  await exerciseRuntimePreparation(page,origin,repositoryId);
  await exerciseDefaultBranchRecheck(page,origin,repositoryId);
  await exerciseRepositoryOnboarding(page,origin,repositoryId);
 } finally {
  await browser?.close();
  if(child&&child.exitCode===null){process.kill(-child.pid,'SIGTERM');await once(child,'exit');}
  await new Promise(resolve=>gateway.close(resolve));
 }
});
