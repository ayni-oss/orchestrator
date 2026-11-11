import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {setTimeout as delay} from 'node:timers/promises';
const namespace=process.env.AYNI_ORCHESTRATOR_NAMESPACE;
assert.ok(namespace==='ayni-orchestrator-capture'||namespace?.startsWith('ayni-orchestrator-acceptance-'));
const gateway=process.env.AYNI_DEBUG_GATEWAY??'http://127.0.0.1:3002';assert.equal(new URL(gateway).hostname,'127.0.0.1');
const kube=(...args)=>execFileSync('kubectl',['--context','docker-desktop','-n',namespace,...args],{encoding:'utf8'});
async function rpc(method,body={}){const response=await fetch(`${gateway}/api/${method}`,{method:'POST',headers:{'x-console-request':'1','content-type':'application/json'},body:JSON.stringify(body)});assert.equal(response.status,200);return response.json();}
async function sources(){let afterId='';const all=[];do{const page=await rpc('ListLogSources',{afterId});all.push(...page.sources??[]);afterId=page.nextAfterId??'';}while(afterId);return all;}
async function eventually(check){for(let i=0;i<120;i++){const result=await check();if(result)return result;await delay(1000);}throw Error('recovery fixture timed out');}
assert.equal((await rpc('GetDebugCapabilities')).enabled,true);
const image=JSON.parse(kube('get','deployment/control-plane','-o','json')).spec.template.spec.containers[0].image;
const suffix=Date.now().toString(36), crash=`debug-crash-${suffix}`, oom=`debug-oom-${suffix}`, force=`debug-force-${suffix}`;
const labels={'app.kubernetes.io/name':'ayni-debug-recovery','app.kubernetes.io/part-of':'ayni-orchestrator','ayni.build/deployment':namespace};
function create(name,command,restartPolicy='Never'){execFileSync('kubectl',['--context','docker-desktop','-n',namespace,'create','-f','-'],{input:JSON.stringify({apiVersion:'v1',kind:'Pod',metadata:{name,labels},spec:{automountServiceAccountToken:false,restartPolicy,containers:[{name:'main',image,command:['/bin/sh','-c',command],resources:{limits:{memory:'32Mi',cpu:'100m'}},securityContext:{allowPrivilegeEscalation:false,capabilities:{drop:['ALL']}}}]}}),stdio:['pipe','pipe','pipe']});}
try {
create(crash,"printf 'restart-output\\n'; sleep 1; exit 42",'OnFailure');
create(oom,'s=x; while :; do s=$s$s; done');
create(force,"printf 'before-forced-delete\\n'; sleep 300");
 await eventually(()=>JSON.parse(kube('get','pod',crash,'-o','json')).status.containerStatuses?.some(s=>s.restartCount>=2));
 await eventually(()=>JSON.parse(kube('get','pod',oom,'-o','json')).status.containerStatuses?.some(s=>s.state?.terminated?.reason==='OOMKilled'));
 const registered=await eventually(async()=>{const all=await sources();return all.some(s=>s.podName===crash&&s.restart>=2)&&all.some(s=>s.podName===force)&&all.some(s=>s.podName===oom)&&all;});
 assert.ok(registered.filter(s=>s.podName===crash).length>=2);
 const before=registered.find(s=>s.podName===force);
 kube('delete','pod',force,'--grace-period=0','--force','--wait=false');
 await eventually(async()=>{const source=(await sources()).find(s=>s.id===before.id);return source?.captureStatus?.includes('deleted');});
 const previous=await sources();
 kube('delete','pod','log-collector-0','--wait=true');
 await eventually(()=>JSON.parse(kube('get','pod','log-collector-0','-o','json')).status?.containerStatuses?.some(s=>s.ready));
 const after=await sources();
 for(const source of previous.filter(s=>[crash,oom,force].includes(s.podName)))assert.ok(after.some(s=>s.id===source.id));
 assert.ok(after.filter(s=>s.podName===crash).every(s=>s.captureStatus!=='complete'));
 console.log(JSON.stringify({namespace,checks:['crash restart inventory','OOM observation','forced deletion uncertainty','collector restart retains history']}));
} finally {for(const name of [crash,oom,force])kube('delete','pod',name,'--ignore-not-found','--wait=false');}
