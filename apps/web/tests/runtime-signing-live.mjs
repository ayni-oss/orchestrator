import assert from 'node:assert/strict';
import {randomUUID} from 'node:crypto';
import {chromium} from 'playwright';
// Explicit opt-in, actual local stack only. Every created key uses public RFC8032
// fixture material or a fresh generated key in the caller's disposable deployment.
const origin=process.env.AYNI_SIGNER_SETUP_UI_ORIGIN;
if(!origin||!/^http:\/\/127\.0\.0\.1:[0-9]+$/.test(origin))throw new Error('Set AYNI_SIGNER_SETUP_UI_ORIGIN to the disposable local UI origin.');
const seed='9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60';
const expectedPublic='d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a';
function publicDiagnostic(mode,httpStatus,body){
 const operation=body?.operation||{};
 const uuid=value=>typeof value==='string'&&/^[0-9a-f-]{36}$/.test(value)?value:'';
 const states=['pending','awaiting_material','completed','blocked'];
 const reasons=['','signer_provisioning_inputs_required','signer_provisioning_disabled','signer_provisioning_unavailable','signer_provisioning_access_denied','signer_provisioning_conflict'];
 return {mode,httpStatus,operationId:uuid(operation.id),idempotencyKey:uuid(operation.idempotencyKey),state:states.includes(operation.state)?operation.state:'unknown',safeReason:reasons.includes(operation.safeReason||'')?operation.safeReason||'':'unknown'};
}
let phase='launch';let activeMode='';let lastPublicDiagnostic=null;
function checkpoint(value){phase=value;console.error(JSON.stringify({phase,mode:activeMode}));}
const browser=await chromium.launch({headless:true});
try{
 const page=await browser.newPage();const results=[];
 checkpoint('open_setup');
 await page.goto(`${origin}/settings/runtime-signing`);await page.waitForFunction(()=>document.documentElement.dataset.hydrated==='true');
 for(const mode of ['generate','import']){
  activeMode=mode;checkpoint('fill_public_intent');
  await page.getByRole('button',{name:'Add signing key',exact:true}).click();
  const keyId=`qualification-${mode}-${randomUUID().slice(0,8)}`;const displayName=`Synthetic signer ${keyId}`;
  await page.getByLabel('Name',{exact:true}).fill(displayName);await page.getByLabel('Key ID',{exact:true}).fill(keyId);
  if(mode==='import'){
   await page.getByRole('radio',{name:'Import an existing key',exact:true}).check();
   await page.getByLabel('Signing seed file',{exact:true}).setInputFiles({name:'public-rfc8032-seed.txt',mimeType:'text/plain',buffer:Buffer.from(seed+'\n')});
   await page.waitForFunction(expected=>document.querySelector('#signer-seed')?.value===expected,seed);
  }
  let accepted;let submitted;let rejected;let calls=0;
  const finished=new Promise((resolve,reject)=>{submitted=resolve;rejected=reject;});
  await page.route('**/api/ProvisionRuntimeSigner',async route=>{
   calls++;
   try{
    const response=await route.fetch();let body;
    try{body=await response.json();}catch{console.error(JSON.stringify(publicDiagnostic(mode,response.status(),null)));throw new Error('Setup returned an unreadable public response.');}
    if(response.status()!==200||body.operation?.state!=='completed'||!body.signer){console.error(JSON.stringify(publicDiagnostic(mode,response.status(),body)));throw new Error('Setup did not complete.');}
    assert.equal(JSON.stringify(body).includes(seed),false);
    lastPublicDiagnostic=publicDiagnostic(mode,response.status(),body);accepted=body;await route.abort('failed');submitted();
   }catch{await route.abort('failed').catch(()=>{});rejected(new Error('Actual signer setup did not return a completed public identity.'));}
  });
  checkpoint('submit_lost_ack');
  await page.getByRole('button',{name:mode==='generate'?'Generate signer':'Import signer',exact:true}).click();await finished;
  checkpoint('check_private_clearing');
  await page.getByRole('region',{name:'Recover signer setup',exact:true}).waitFor();
  const stored=await page.evaluate(()=>[...Object.values(sessionStorage),...Object.values(localStorage)].join('\n'));
  assert.equal(stored.includes(seed),false);assert.equal(stored.includes('signingSeed'),false);
  assert.equal((await page.locator('body').innerText()).includes(seed),false);
  assert.equal(await page.locator('#signer-seed').count(),0);
  checkpoint('reload_recovery');
  await page.unroute('**/api/ProvisionRuntimeSigner');await page.reload();
  const card=page.getByRole('region',{name:displayName,exact:true});await card.waitFor();
  // ListRuntimeSigners can display the saved card before onMount recovery has
  // completed. Wait for the recovery's state transition, not merely its list row.
  await page.waitForFunction(()=>sessionStorage.getItem('ayni.runtime.signer.provisioning.pending')===null);
  await page.getByRole('button',{name:'Add signing key',exact:true}).waitFor();
  checkpoint('recovery_accepted');
  assert.equal(calls,1);assert.equal(await page.getByRole('button',{name:'Generate signer',exact:true}).count(),0);
  assert.equal(await page.evaluate(()=>sessionStorage.getItem('ayni.runtime.signer.provisioning.pending')),null);
  const signer=accepted.signer;
  assert.equal(signer.keyId,keyId);assert.match(signer.publicKey,/^[0-9a-f]{64}$/);assert.ok(signer.secretUid);
  if(mode==='import')assert.equal(signer.publicKey,expectedPublic);
  checkpoint('verify_repeated_public_receipt');
  for(let retry=0;retry<2;retry++){
   const response=await page.request.post(`${origin}/api/RecoverRuntimeSignerProvisioning`,{headers:{'x-console-request':'1'},data:{idempotencyKey:accepted.operation.idempotencyKey}});
   assert.equal(response.status(),200);const recovered=await response.json();
   assert.equal(JSON.stringify(recovered).includes(seed),false);assert.equal(recovered.signer.secretUid,signer.secretUid);assert.equal(recovered.signer.id,signer.id);assert.equal(recovered.signer.publicKey,signer.publicKey);
  }
  results.push({mode,id:signer.id,keyId,publicKey:signer.publicKey,secretName:signer.secretName,secretUid:signer.secretUid,operationId:accepted.operation.id,idempotencyKey:accepted.operation.idempotencyKey});
 }
 await page.screenshot({path:'/tmp/runtime-signing-actual-stack.png',fullPage:true});
 console.log(JSON.stringify({state:'passed',boundary:'actual-stack-synthetic-signers',origin,signers:results}));
}catch{
 console.error(JSON.stringify({failedPhase:phase,mode:activeMode,...(lastPublicDiagnostic||{})}));
 console.error('Actual signer setup qualification failed; inspect public operation status and the disposable deployment.');process.exitCode=1;
}finally{await browser.close();}
