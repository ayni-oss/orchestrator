import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createServer as createSocket} from 'node:net';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {randomUUID} from 'node:crypto';
import {setTimeout as delay} from 'node:timers/promises';
import {chromium} from 'playwright';
const seed='9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60';
const otherSeed='2'.repeat(64);
const publicKey='d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a';
// Synthetic HTTP authority: this test establishes UI recovery and privacy only.
// Real immutable Secret creation is qualified separately against Kubernetes.
test('multiple named signers survive reload, edit independently and recover without private material',{timeout:180000},async()=>{
 const signers=[];const operations=new Map();const requests=[];let generated=0;let imported=0;let disabled=false;let failList=false;let failedList=false;let releaseImport;let rejectNext=false;
 function complete(operation){
  if(operation.state!=='completed'){
   operation.state='completed';operation.safeReason='';operation.publicKey=publicKey;
   operation.secretUid=randomUUID();operation.signerId=randomUUID();
   operation.signer={id:operation.signerId,displayName:operation.displayName,keyId:operation.keyId,publicKey,secretName:operation.secretName,secretUid:operation.secretUid,revision:'1',activeRevision:randomUUID(),enabled:true};
   signers.push(operation.signer);
   if(operation.mode==='generate')generated++;else imported++;
  }
  return {operation,signer:operation.signer};
 }
 const gateway=createServer(async(request,response)=>{
  let body='';for await(const bytes of request)body+=bytes;
  const data=body?JSON.parse(body):{};requests.push({method:request.url,data});
  function send(value,status=200){response.writeHead(status,{'Content-Type':'application/json'});response.end(JSON.stringify(value));}
  if(request.url==='/api/ListRuntimeSigners'){if(failList){failList=false;return send({message:'synthetic list unavailable'},503);}return send({signers});}
  if(request.url==='/api/SaveRuntimeSigner'){
   const index=signers.findIndex(value=>value.id===data.id);const current=signers[index];
   if(!current||current.revision!==data.expectedRevision)return send({message:'stale signer'},409);
   const signer={...current,...data,revision:String(Number(current.revision)+1),activeRevision:randomUUID()};
   signers[index]=signer;return send({signer});
  }
  if(request.url==='/api/ProvisionRuntimeSigner'){
   if(rejectNext){rejectNext=false;return send({message:'synthetic public validation rejection'},400);}
   let operation=operations.get(data.idempotencyKey);
   if(!operation){operation={id:randomUUID(),idempotencyKey:data.idempotencyKey,displayName:data.displayName,keyId:data.keyId,mode:data.mode,secretName:`ayni-runtime-signer-${randomUUID()}`,state:'pending',safeReason:'',publicKey:'',secretUid:'',signerId:''};operations.set(data.idempotencyKey,operation);}
   if(disabled){operation.safeReason='signer_provisioning_disabled';return send({operation});}
   if(data.mode==='generate')return send(complete(operation));
   if(data.signingSeed!==seed)return send({message:`signer_provisioning_conflict ${data.signingSeed}`},409);
   if(operation.state==='pending'){operation.state='awaiting_material';operation.safeReason='signer_provisioning_inputs_required';await new Promise(resolve=>{releaseImport=resolve;gateway.emit('import-pending');});return send({message:`interrupted import ${data.signingSeed}`},503);}
   return send(complete(operation));
  }
  if(request.url==='/api/RecoverRuntimeSignerProvisioning'){
   const operation=operations.get(data.idempotencyKey);if(!operation)return send({message:'not found'},404);
   if(operation.state==='completed'&&operation.mode==='generate'&&!failedList){failList=true;failedList=true;await delay(150);}
   return send(operation.state==='completed'?complete(operation):{operation});
  }
  return send(request.url==='/api/ListWorkspaces'?{workspaces:[]}:{});
 });
 let child,browser;
 try{
  gateway.listen(0,'127.0.0.1');await once(gateway,'listening');
  const socket=createSocket();socket.listen(0,'127.0.0.1');await once(socket,'listening');const port=socket.address().port;await new Promise(resolve=>socket.close(resolve));
  const origin=`http://127.0.0.1:${port}`;const env={...process.env,AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT:String(gateway.address().port),HOST:'127.0.0.1',PORT:String(port)};
  child=process.env.AYNI_PRODUCT_PRODUCTION==='1'?spawn('node',['apps/web/build/index.js'],{env,stdio:'ignore',detached:true}):spawn('pnpm',['--dir','apps/web','exec','vite','dev','--host','127.0.0.1','--port',String(port),'--strictPort'],{env,stdio:'ignore',detached:true});
  let ready=false;for(let i=0;i<120;i++){try{if((await fetch(origin)).ok){ready=true;break;}}catch{}await delay(250);}assert.ok(ready,'UI fixture server did not start');
  browser=await chromium.launch({headless:true});const page=await browser.newPage();
  const visit=async()=>{await page.goto(`${origin}/settings/runtime-signing`);await page.waitForFunction(()=>document.documentElement.dataset.hydrated==='true');await page.getByRole('button',{name:'Refresh signers',exact:true}).waitFor();};
  const fill=async(_label,key)=>{await page.getByLabel('Name',{exact:true}).fill(key);};
  const privateAbsent=async()=>{
   const stored=await page.evaluate(()=>[...Object.values(localStorage),...Object.values(sessionStorage)].join('\n'));
   const inputValues=await page.locator('input').evaluateAll(inputs=>inputs.map(input=>input.value));
   assert.equal(stored.includes('signingSeed'),false);
   for(const value of [seed,otherSeed]){assert.equal(inputValues.some(input=>input.includes(value)),false);assert.equal(stored.includes(value),false);assert.equal((await page.locator('body').innerText()).includes(value),false);}
   for(const request of requests.filter(value=>value.method==='/api/RecoverRuntimeSignerProvisioning'))assert.deepEqual(Object.keys(request.data),['idempotencyKey']);
  };
  await visit();assert.equal(await page.getByLabel('Name',{exact:true}).count(),0);await page.getByRole('button',{name:'Add signing key',exact:true}).click();assert.equal(await page.getByRole('radio',{name:'Generate a new key',exact:true}).isChecked(),true);
  await fill('generated.fixture','generated.fixture');
  await page.route('**/api/ProvisionRuntimeSigner',async route=>{await route.fetch();await route.abort('failed');});
  await page.getByRole('button',{name:'Generate signer',exact:true}).click();
  await page.getByRole('region',{name:'Recover signer setup',exact:true}).waitFor();
  await page.getByText(/setup response was interrupted/).waitFor();
  assert.equal(generated,1);assert.equal(signers.length,1);
  const retained=JSON.parse(await page.evaluate(()=>sessionStorage.getItem('ayni.runtime.signer.provisioning.pending')));
  assert.deepEqual(Object.keys(retained).sort(),['displayName','idempotencyKey','keyId','mode']);
  await privateAbsent();await page.unroute('**/api/ProvisionRuntimeSigner');await page.reload();
  const generatedCard=page.getByRole('region',{name:'generated.fixture',exact:true});await generatedCard.waitFor();
  // The list card can precede receipt acceptance; wait for recovery to commit.
  await page.waitForFunction(()=>sessionStorage.getItem('ayni.runtime.signer.provisioning.pending')===null);
  await page.getByRole('button',{name:'Add signing key',exact:true}).waitFor();
  assert.equal(await page.getByRole('button',{name:'Generate signer',exact:true}).count(),0);
  assert.equal(generated,1);assert.equal(requests.filter(value=>value.method==='/api/ProvisionRuntimeSigner').length,1);
  await generatedCard.locator('summary').click();await generatedCard.getByText('"generated.fixture" = "'+publicKey+'"',{exact:false}).waitFor();
  assert.equal(await page.evaluate(()=>sessionStorage.getItem('ayni.runtime.signer.provisioning.pending')),null);
  await page.getByRole('button',{name:'Add signing key',exact:true}).click();await page.getByRole('radio',{name:'Import an existing key',exact:true}).check();await fill('imported.fixture','imported.fixture');
  await page.getByLabel('Private signing seed',{exact:true}).fill(seed);
  const importPending=once(gateway,'import-pending');
  await page.getByRole('button',{name:'Import signer',exact:true}).click();await importPending;
  assert.equal(await page.locator('#signer-seed').count(),0);await privateAbsent();releaseImport();releaseImport=undefined;
  await page.getByText(/signer setup service is unavailable/).waitFor();
  assert.equal(await page.locator('#signer-seed').count(),0);await privateAbsent();
  await page.reload();await page.getByLabel('Same private signing seed',{exact:true}).waitFor();
  await page.getByLabel('Same private signing seed',{exact:true}).fill(otherSeed);await page.getByRole('button',{name:'Resume signer setup',exact:true}).click();
  await page.getByText(/retained signing identity or Secret conflicts/).waitFor();assert.equal(await page.getByLabel('Same private signing seed',{exact:true}).inputValue(),'');await privateAbsent();
  const beforeInvalid=requests.filter(value=>value.method==='/api/ProvisionRuntimeSigner').length;
  await page.getByLabel('Signing seed file',{exact:true}).setInputFiles({name:'invalid-seed.txt',mimeType:'text/plain',buffer:Buffer.from(seed+'\n\n')});
  await page.getByText(/Choose a raw signing seed file/).waitFor();assert.equal(await page.getByLabel('Signing seed file',{exact:true}).inputValue(),'');assert.equal(requests.filter(value=>value.method==='/api/ProvisionRuntimeSigner').length,beforeInvalid);
  await page.getByLabel('Signing seed file',{exact:true}).setInputFiles({name:'synthetic-seed.txt',mimeType:'text/plain',buffer:Buffer.from(seed+'\n')});
  await page.waitForFunction(expected=>document.querySelector('#signer-seed')?.value===expected,seed);
  await page.getByRole('button',{name:'Resume signer setup',exact:true}).click();
  const importedCard=page.getByRole('region',{name:'imported.fixture',exact:true});await importedCard.waitFor();assert.equal(imported,1);await privateAbsent();
  await page.reload();await generatedCard.waitFor();await importedCard.waitFor();
  await page.getByRole('button',{name:'Edit generated.fixture',exact:true}).click();assert.equal(await page.getByLabel('Name',{exact:true}).inputValue(),'generated.fixture');
  await page.getByRole('button',{name:'Cancel',exact:true}).click();assert.equal(await page.getByLabel('Name',{exact:true}).count(),0);
  assert.equal(await page.evaluate(()=>sessionStorage.getItem('ayni.runtime.signer.provisioning.pending')),null);
  await page.getByRole('button',{name:'Edit generated.fixture',exact:true}).click();
  await page.getByLabel('Name',{exact:true}).fill('project-2026');
  assert.equal(await page.getByLabel('Key ID',{exact:true}).count(),0,'the name is the trust identity, not an independent label');
  await page.getByRole('button',{name:'Save signer reference',exact:true}).click();
  const namedCard=page.getByRole('region',{name:'project-2026',exact:true});await namedCard.waitFor();
  const renamed=requests.filter(value=>value.method==='/api/SaveRuntimeSigner').at(-1).data;
  assert.equal(renamed.keyId,'project-2026');assert.equal(renamed.displayName,'project-2026');assert.equal(renamed.publicKey,publicKey);
  await namedCard.locator('summary').click();await namedCard.getByText('"project-2026" = "'+publicKey+'"',{exact:false}).waitFor();
  await page.getByRole('button',{name:'Add signing key',exact:true}).click();
  const beforeValidation=requests.filter(value=>value.method==='/api/ProvisionRuntimeSigner').length;
  await fill('Reserved key','unsigned');await page.getByRole('button',{name:'Generate signer',exact:true}).click();await page.getByText(/name unsigned is reserved/).waitFor();
  await fill('   ','   ');await page.getByRole('button',{name:'Generate signer',exact:true}).click();await page.getByText(/Enter a shorter name/).waitFor();
  assert.equal(requests.filter(value=>value.method==='/api/ProvisionRuntimeSigner').length,beforeValidation);
  assert.equal(await page.evaluate(()=>sessionStorage.getItem('ayni.runtime.signer.provisioning.pending')),null);
  rejectNext=true;await fill('Rejected public name','rejected.fixture');await page.getByRole('button',{name:'Generate signer',exact:true}).click();await page.getByText(/signing input was rejected/).waitFor();
  const rejectedIntent=JSON.parse(await page.evaluate(()=>sessionStorage.getItem('ayni.runtime.signer.provisioning.pending')));
  assert.equal(await page.getByLabel('Name',{exact:true}).isDisabled(),true);
  await page.getByRole('button',{name:'Recover signer setup',exact:true}).click();await page.getByText(/rejected setup has no receipt/).waitFor();
  await fill('corrected.fixture','corrected.fixture');await page.getByRole('button',{name:'Resume signer setup',exact:true}).click();await page.getByRole('region',{name:'corrected.fixture',exact:true}).waitFor();
  const correctedRequest=requests.filter(value=>value.method==='/api/ProvisionRuntimeSigner').at(-1).data;assert.equal(correctedRequest.idempotencyKey,rejectedIntent.idempotencyKey);assert.equal(correctedRequest.keyId,'corrected.fixture');
  await page.getByRole('button',{name:'Add signing key',exact:true}).click();await fill('Disabled setup','disabled.fixture');disabled=true;
  await page.getByRole('button',{name:'Generate signer',exact:true}).click();await page.getByText('Signer setup is disabled in this deployment. Ask the administrator to enable it, then recover this setup.',{exact:true}).waitFor();
  assert.equal(generated,2);assert.equal(imported,1);await privateAbsent();
  assert.equal(requests.some(value=>/RuntimePreparation|StartTask/.test(value.method)),false);
  await page.screenshot({path:'/tmp/runtime-signing-setup-browser.png',fullPage:true});
 }finally{
  releaseImport?.();await browser?.close();if(child&&child.exitCode===null){process.kill(-child.pid,'SIGTERM');await once(child,'exit');}await new Promise(resolve=>gateway.close(resolve));
 }
});
