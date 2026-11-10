import test from 'node:test';
import assert from 'node:assert/strict';
import {runtimeOverview,runtimeRegistryImage} from '../src/lib/runtime-preparation.ts';
const inspection={id:'inspection',state:'completed',commitSha:'a'.repeat(40),connectionRevision:'1',result:{trustKeyIds:['trusted'],findings:[]}};
const base={loaded:true,error:'',pending:false,inspectionLoading:false,inspectionError:'',inspection,operations:[],registries:[{enabled:true,activeCredentialRevision:'1'}],signers:[{enabled:true,keyId:'trusted'}],sourceReady:true,sourceRevision:'1'};

test('runtime overview explains absent preparation and known configuration blockers',()=>{
 const clean=runtimeOverview(base);
 assert.equal(clean.title,'Runtime not prepared');
 assert.match(clean.nextAction,/Configure a runtime/);
 const blocked=runtimeOverview({...base,registries:[],signers:[{enabled:true,keyId:'other'}],inspection:{...inspection,result:{trustKeyIds:['trusted'],findings:[{code:'ayni_version_unsupported',action:'Regenerate the lock with supported ayni.'},{code:'adapter_discovery_pending',action:'Discovery still required.'}]}}});
 assert.equal(blocked.title,'Runtime setup needs attention');
 assert.deepEqual(blocked.issues,['Regenerate the lock with supported ayni.','No enabled, validated runtime registry is available. Configure a registry connection.','The inspection trusts key names: trusted. Enabled signing keys are named: other. Use the same key name and public key in the repository trust policy.']);
});

test('runtime readiness stays scoped to the inspected commit and platform',()=>{
 const ready={id:'ready',inspectionId:'old',state:'ready',platform:'linux/arm64',stage:'cleanup',sourceCommit:'b'.repeat(40)};
 assert.equal(runtimeOverview({...base,operations:[ready]}).title,'Runtime not prepared');
 const current=runtimeOverview({...base,operations:[{...ready,inspectionId:inspection.id}]});
 assert.equal(current.title,'Runtime ready');
 assert.equal(runtimeOverview({...base,operations:[{...ready,inspectionId:inspection.id,developmentOnly:true}]}).title,'Development runtime ready');
 assert.match(current.description,/bbbbbbbbbbbb · linux\/arm64/);
 assert.match(current.nextAction,/exact inspection and platform/);
 const olderActive=runtimeOverview({...base,operations:[{...ready,state:'running',stage:'build'}]});
 assert.equal(olderActive.title,'Runtime preparation in progress');assert.match(olderActive.description,/bbbbbbbbbbbb/);
});

test('unknown and interrupted responses never look like an unprepared or ready runtime',()=>{
 assert.equal(runtimeOverview({...base,loaded:false}).title,'Checking runtime status');
 assert.equal(runtimeOverview({...base,error:'History unavailable'}).title,'Runtime status unavailable');
 assert.equal(runtimeOverview({...base,inspectionError:'Inspection unavailable'}).title,'Runtime status unavailable');
 assert.equal(runtimeOverview({...base,inspectionPending:true}).title,'Inspection result needs recovery');
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


test('registry links expose confirmed immutable images without treating intended publication as success',()=>{
 assert.equal(runtimeRegistryImage(undefined,'ghcr.io/ayni-oss/sandbox').url,'https://github.com/orgs/ayni-oss/packages/container/package/sandbox');
 assert.equal(runtimeRegistryImage(undefined,'https://untrusted.example/anything'),null);
 const digest=`sha256:${'a'.repeat(64)}`;
 const pending={registryDestination:'ghcr.io/ayni-oss/sandbox',imageDigest:digest,expectedManifestDigest:digest,state:'publication_uncertain',stage:'publish'};
 assert.equal(runtimeRegistryImage(pending).confirmed,false);
 assert.equal(runtimeRegistryImage({...pending,state:'running',stage:'pull_final'}).reference,`ghcr.io/ayni-oss/sandbox@${digest}`);
 assert.equal(runtimeRegistryImage({...pending,state:'cleanup_pending',stage:'cleanup',imageDigest:`sha256:${'b'.repeat(64)}`}).confirmed,false);
});
