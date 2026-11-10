import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createServer as createSocket} from 'node:net';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {randomUUID} from 'node:crypto';
import {setTimeout as delay} from 'node:timers/promises';
import {chromium} from 'playwright';
import {exerciseRuntimePreparation,exerciseDefaultBranchRecheck,exerciseRepositoryOnboarding,exerciseRepositoryHistoryPagination} from './runtime-preparation-ui.mjs';

// These fixtures exercise browser behavior with intercepted provider responses.
async function withRepositoryUi(exercise){
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
  await exercise(page,origin,repositoryId);
 } finally {
  await browser?.close();
  if(child&&child.exitCode===null){process.kill(-child.pid,'SIGTERM');await once(child,'exit');}
  await new Promise(resolve=>gateway.close(resolve));
 }
}

test('runtime preparation UI retains explicit grants and recovers exact commands', {timeout:120000}, ()=>withRepositoryUi(async(page,origin,repositoryId)=>{
 await exerciseRuntimePreparation(page,origin,repositoryId);
 await exerciseDefaultBranchRecheck(page,origin,repositoryId);
 await exerciseRepositoryOnboarding(page,origin,repositoryId);
}));

test('repository history pagination caches batches of ten and shows five at a time', {timeout:60000}, ()=>withRepositoryUi(exerciseRepositoryHistoryPagination));

test('delayed repository reads preserve the page structure and refresh keeps rows visible', {timeout:60000}, ()=>withRepositoryUi(async(page,origin,repositoryId)=>{
 await page.setViewportSize({width:724,height:853});
 let releaseCatalog,releaseHistory;
 const catalogGate=new Promise(resolve=>{releaseCatalog=resolve;});
 const historyGate=new Promise(resolve=>{releaseHistory=resolve;});
 const operation={id:randomUUID(),repositoryId,platform:'linux/arm64',state:'ready',stage:'cleanup',updatedAtUnixMs:String(Date.now()),contentVerified:true,managedCompatible:true,cleanupComplete:true};
 await page.route('**/api/ListRepositories',async route=>{await catalogGate;await route.continue();});
 let historyHeld=true;
 await page.route('**/api/ListRuntimePreparations',async route=>{if(historyHeld)await historyGate;await route.fulfill({json:{operations:[operation]}});});
 await page.goto(`${origin}/repositories/${repositoryId}?tab=status`);
 const inspect=page.getByRole('heading',{name:'Inspect a specific commit',exact:true});
 await inspect.waitFor();
 const tab=page.getByRole('tab',{name:'Status',exact:true});
 const beforeTab=await tab.boundingBox(),beforeForm=await inspect.boundingBox();

 assert.equal(await page.getByRole('heading',{name:'Current preparation details',exact:true}).count(),0);
 assert.ok(await page.locator('[data-slot=skeleton]').count()>0);
 assert.equal(await page.getByRole('button',{name:'Inspect commit',exact:true}).isDisabled(),true);
 releaseCatalog();
 await page.getByRole('heading',{name:'fixture/runtime-ui',exact:true}).waitFor();
 const current=page.getByRole('region',{name:'Current preparation details',exact:true});
 await current.waitFor();
 assert.ok(await current.locator('[data-slot=skeleton]').count()>0);
 historyHeld=false;releaseHistory();
 const row=current.getByRole('article');
 await row.waitFor();
 const afterTab=await tab.boundingBox(),afterForm=await inspect.boundingBox();
 assert.ok(Math.abs(beforeTab.y-afterTab.y)<2,`tabs moved ${beforeTab.y} → ${afterTab.y}`);
 assert.ok(Math.abs(beforeForm.y-afterForm.y)<2,`form moved ${beforeForm.y} → ${afterForm.y}`);
 const title=page.locator('header.page-header [data-title-content]');
 assert.equal(await title.getByText('Configured',{exact:true}).count(),1);
 const sourceInfo=page.locator('header.page-header [data-title-actions]').getByRole('button',{name:'View source integration details'});
 await sourceInfo.hover();
 await page.getByRole('tooltip').getByText('Installation ID',{exact:true}).waitFor();
 assert.equal(await page.getByRole('dialog').count(),0);
 assert.equal(await page.getByRole('tooltip').locator('dt').count(),4);
 await page.getByRole('tab',{name:'Status',exact:true}).hover();
 await page.getByRole('tooltip').waitFor({state:'hidden'});
 await sourceInfo.focus();
 await page.getByRole('tooltip').getByText('Installation ID',{exact:true}).waitFor();
 await page.keyboard.press('Escape');
 await page.getByRole('tooltip').waitFor({state:'hidden'});
 let releaseRefresh;
 const refreshGate=new Promise(resolve=>{releaseRefresh=resolve;});
 await page.unroute('**/api/ListRuntimePreparations');
 await page.route('**/api/ListRuntimePreparations',async route=>{await refreshGate;await route.fulfill({json:{operations:[operation]}});});
 const refresh=page.getByRole('button',{name:'Refresh preparation history',exact:true});
 await refresh.click();
 assert.equal(await row.isVisible(),true);
 assert.equal(await current.locator('[data-slot=skeleton]').count(),0);
 releaseRefresh();
 await page.waitForFunction(()=>!document.querySelector('[aria-label="Refresh preparation history"]').disabled);
}));

test('connection settings render their form during loading and retain cards during refresh', {timeout:60000}, ()=>withRepositoryUi(async(page,origin)=>{
 const connection={id:randomUUID(),displayName:'Example GitHub',capability:'source_control',provider:'github',appId:'123',enabled:true,activeCredentialRevision:'credential',revision:'1',status:'configured'};
 let release;
 const gate=new Promise(resolve=>{release=resolve;});
 await page.route('**/api/GetConnection',async route=>{await gate;await route.fulfill({json:{connection}});});
 await page.route('**/api/ListConnectionOperations',route=>route.fulfill({json:{operations:[]}}));
 await page.goto(`${origin}/settings/source-control/github/${connection.id}`);
 const form=page.getByRole('region',{name:'Edit connection',exact:true});
 await form.waitFor();
 assert.equal(await form.getByLabel('Connection name',{exact:true}).isDisabled(),true);
 assert.equal(await page.getByRole('region',{name:'Danger zone',exact:true}).isVisible(),true);
 assert.ok(await page.locator('[data-slot=skeleton]').count()>0);
 release();
 await page.getByRole('heading',{name:'Example GitHub',exact:true}).waitFor();
 const details=page.getByRole('region',{name:'Connection details',exact:true});
 assert.equal(await details.locator('[data-title-content]').getByText('Configured',{exact:true}).count(),1);
 let releaseRefresh;
 const refreshGate=new Promise(resolve=>{releaseRefresh=resolve;});
 await page.unroute('**/api/GetConnection');
 await page.route('**/api/GetConnection',async route=>{await refreshGate;await route.fulfill({json:{connection}});});
 await page.getByRole('button',{name:'Refresh settings',exact:true}).click();
 assert.equal(await details.isVisible(),true);
 assert.equal(await details.locator('[data-slot=skeleton]').count(),0);
 assert.equal(await form.getByLabel('Connection name',{exact:true}).inputValue(),'Example GitHub');
 releaseRefresh();
}));
