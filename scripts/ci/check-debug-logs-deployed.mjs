// Run only against the explicitly authorised local qualification deployment.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {setTimeout as delay} from 'node:timers/promises';
import {chromium} from 'playwright';

const namespace=process.env.AYNI_ORCHESTRATOR_NAMESPACE;
assert.ok(namespace==='ayni-orchestrator-capture'||namespace?.startsWith('ayni-orchestrator-acceptance-'),'a dedicated acceptance namespace is required');
const gateway=process.env.AYNI_DEBUG_GATEWAY??'http://127.0.0.1:3002';
const frontend=process.env.AYNI_DEBUG_FRONTEND??'http://127.0.0.1:5176';
for(const value of [gateway,frontend]) assert.equal(new URL(value).hostname,'127.0.0.1');
const kube=(...args)=>execFileSync('kubectl',['--context','docker-desktop','--namespace',namespace,...args],{encoding:'utf8'});
const call=async(method,body={})=>{const r=await fetch(`${gateway}/api/${method}`,{method:'POST',headers:{'content-type':'application/json','x-console-request':'1'},body:JSON.stringify(body)});assert.equal(r.status,200);return r.json();};
const inventory=async()=>{const all=[];let afterId='';do{const p=await call('ListLogSources',{afterId});all.push(...p.sources??[]);afterId=p.nextAfterId??'';}while(afterId);return all;};
async function eventually(check){for(let i=0;i<120;i++){const value=await check();if(value)return value;await delay(1000);}throw Error('capture acceptance timed out');}
assert.equal((await call('GetDebugCapabilities')).enabled,true);
const deployment=JSON.parse(kube('get','deployment/control-plane','-o','json'));
const image=deployment.spec.template.spec.containers[0].image;
const suffix=Date.now().toString(36), name=`debug-capture-${suffix}`, unrelated=`debug-unrelated-${suffix}`, failed=`debug-init-${suffix}`;
const owned={ 'app.kubernetes.io/name':'ayni-debug-qualification','app.kubernetes.io/part-of':'ayni-orchestrator','ayni.build/deployment':namespace};
const container=(name,script)=>({name,image,command:['/bin/sh','-c',script],securityContext:{allowPrivilegeEscalation:false,readOnlyRootFilesystem:true,capabilities:{drop:['ALL']}}});
const payload="same\nsame\n<img src=x onerror=globalThis.__rawExecuted=true>\u001b[2J";
function create(name,labels,containers,initContainers=[]){const pod={apiVersion:'v1',kind:'Pod',metadata:{name,namespace,labels},spec:{automountServiceAccountToken:false,restartPolicy:'Never',containers,initContainers}};execFileSync('kubectl',['--context','docker-desktop','--namespace',namespace,'create','-f','-'],{input:JSON.stringify(pod),stdio:['pipe','pipe','pipe']});}
let browser;
try {
create(name,owned,[container('main',`printf 'same\\nsame\\n<img src=x onerror=globalThis.__rawExecuted=true>\\033[2J'; printf '\\000\\377tail'`),container('sidecar',"printf 'sidecar-output\\n'")],[container('init',"printf 'init-output\\n'")]);
create(unrelated,{},[container('main',"printf 'unrelated-output\\n'")]);
create(failed,owned,[container('never-started','true')],[container('failing-init',"printf 'init-failure\\n' >&2; exit 42")]);
 const sources=await eventually(async()=>{const s=await inventory();return s.some(s=>s.podName===name&&s.container==='main')&&s.some(s=>s.podName===failed&&s.container==='never-started')&&s;});
 assert.ok(!sources.some(s=>s.podName===unrelated));
 for(const c of ['main','sidecar','init'])assert.ok(sources.some(s=>s.podName===name&&s.container===c));
 const source=sources.find(s=>s.podName===name&&s.container==='main');
 const download=async()=>Buffer.from(await (await fetch(`${gateway}/debug/logs/download?sourceId=${encodeURIComponent(source.id)}`)).arrayBuffer());
 await eventually(async()=>{const bytes=await download();return bytes.includes(Buffer.from(payload))&&bytes.includes(Buffer.from([0,255,116,97,105,108]));});
 const before=await download();
 kube('delete','pod',name,'--wait=true');
 assert.deepEqual(await download(),before,'raw bytes survive pod deletion');
 browser=await chromium.launch({headless:true});const page=await browser.newPage();page.setDefaultTimeout(20000);
 const errors=[];page.on('pageerror',error=>errors.push(error.message));
 await page.goto(`${frontend}/debug/logs`);
 await page.getByLabel('Source ID',{exact:true}).fill(source.id);
 await page.getByRole('button',{name:'Apply filters',exact:true}).click();
 await page.getByRole('button',{name:'Follow live',exact:true}).click();
 await eventually(async()=> (await page.getByRole('region',{name:'Execution logs'}).innerText()).includes('<img src=x onerror='));
 await page.getByRole('button',{name:'Pause',exact:true}).click();
 assert.equal(await page.evaluate(()=>globalThis.__rawExecuted),undefined);
 assert.equal(await page.getByRole('region',{name:'Execution logs'}).locator('img').count(),0);
 assert.equal(await page.evaluate(()=>localStorage.length),0);
 assert.deepEqual(errors,[]);
 console.log(JSON.stringify({namespace,sourceCount:sources.length,checks:['init and sidecar inventory','failed-to-start inventory','unrelated exclusion','identical lines','binary bytes','retention after deletion','frontend follow/pause','inert HTML and controls']}));
} finally {
 await browser?.close();
 for(const pod of [name,unrelated,failed])kube('delete','pod',pod,'--ignore-not-found','--wait=false');
}
