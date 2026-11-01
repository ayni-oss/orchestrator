import test from 'node:test';
import assert from 'node:assert/strict';
import {runtimeOverview} from '../src/lib/runtime-preparation.ts';
const inspection={id:'inspection',state:'completed',commitSha:'a'.repeat(40),connectionRevision:'1',result:{trustKeyIds:['trusted'],findings:[]}};
const base={loaded:true,error:'',pending:false,inspectionLoading:false,inspectionError:'',inspection,operations:[],registries:[{enabled:true,activeCredentialRevision:'1'}],signers:[{enabled:true,keyId:'trusted'}],sourceReady:true,sourceRevision:'1'};

test('runtime overview explains absent preparation and known configuration blockers',()=>{
 const clean=runtimeOverview(base);
 assert.equal(clean.title,'Runtime not prepared');
 assert.match(clean.nextAction,/Select the inspection/);
 const blocked=runtimeOverview({...base,registries:[],signers:[{enabled:true,keyId:'other'}],inspection:{...inspection,result:{trustKeyIds:['trusted'],findings:[{code:'ayni_version_unsupported',action:'Regenerate the lock with supported Ayni.'},{code:'adapter_discovery_pending',action:'Discovery still required.'}]}}});
 assert.equal(blocked.title,'Runtime setup needs attention');
 assert.deepEqual(blocked.issues,['Regenerate the lock with supported Ayni.','No enabled, validated runtime registry is available. Configure a registry connection.','The inspection trusts Key IDs: trusted. Enabled signers use: Signer (Key ID other). The Key ID and public key must both match the repository trust policy; a matching display name is not enough.']);
});

test('runtime readiness stays scoped to the inspected commit and platform',()=>{
 const ready={id:'ready',inspectionId:'old',state:'ready',platform:'linux/arm64',stage:'cleanup',sourceCommit:'b'.repeat(40)};
 assert.equal(runtimeOverview({...base,operations:[ready]}).title,'Runtime not prepared');
 const current=runtimeOverview({...base,operations:[{...ready,inspectionId:inspection.id}]});
 assert.equal(current.title,'Runtime ready');
 assert.match(current.description,/bbbbbbbbbbbb · linux\/arm64/);
 assert.match(current.nextAction,/exact inspection and platform/);
});

test('unknown and interrupted responses never look like an unprepared or ready runtime',()=>{
 assert.equal(runtimeOverview({...base,loaded:false}).title,'Checking runtime status');
 assert.equal(runtimeOverview({...base,error:'History unavailable'}).title,'Runtime status unavailable');
 assert.equal(runtimeOverview({...base,inspectionError:'Inspection unavailable'}).title,'Runtime status unavailable');
 assert.equal(runtimeOverview({...base,pending:true,error:'Lost response'}).title,'Preparation response needs recovery');
 assert.equal(runtimeOverview({...base,pending:true,rejected:true,error:'Selection rejected'}).title,'Preparation request rejected');
 assert.equal(runtimeOverview({...base,moreHistory:true}).title,'More runtime history available');
});

test('saved failure, publication and cleanup states explain recovery without claiming readiness',()=>{
 for(const [state,safeReason,title] of [['failed','registry_access_denied','Runtime preparation failed'],['publication_uncertain','publication_uncertain','Publication needs reconciliation'],['cleanup_pending','cleanup_pending','Runtime cleanup pending']]){
  const result=runtimeOverview({...base,operations:[{inspectionId:inspection.id,state,safeReason,stage:state==='cleanup_pending'?'cleanup':'publish',platform:'linux/arm64'}]});
  assert.equal(result.title,title);assert.ok(result.issues.length);assert.ok(result.nextAction);
 }
 assert.match(runtimeOverview({...base,sourceRevision:'2'}).issues.join(' '),/source connection changed/);
 assert.ok(runtimeOverview({...base,inspection:{...inspection,state:'failed'},inspectionReason:'The saved App key is invalid.'}).issues.includes('The saved App key is invalid.'));
});
