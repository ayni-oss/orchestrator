import { createFixture, origin } from './ui-fixture.mjs';
import test from 'node:test';
import assert from 'node:assert/strict';
import {once} from 'node:events';
import {randomUUID} from 'node:crypto';
import {setTimeout as delay} from 'node:timers/promises';
import {chromium} from 'playwright';
import {exerciseRuntimePreparation,exerciseDefaultBranchRecheck,exerciseRepositoryOnboarding,exerciseRepositoryHistoryPagination} from './runtime-preparation-ui.mjs';

// These fixtures exercise browser behavior with intercepted provider responses.
async function withRepositoryUi(exercise){
 const repositoryId=randomUUID();
 const repository={id:repositoryId,connectionId:randomUUID(),installationId:'123',providerRepositoryId:'456',fullName:'fixture/runtime-ui',defaultBranch:'main',private:true,connectedAtUnixMs:String(Date.now())};
 const gateway=createFixture((request,response)=>{
  const body=request.url==='/api/ListRepositories'?{repositories:[repository]}:request.url==='/api/ListWorkspaces'?{workspaces:[]}:request.url==='/api/ListConnections'?{connections:[{id:repository.connectionId,displayName:'Fixture GitHub',capability:'source_control',enabled:true,activeCredentialRevision:'credential',revision:'1'}]}:{};
  response.writeHead(200,{'Content-Type':'application/json'});response.end(JSON.stringify(body));
 });
 let browser;
 try {
  browser=await chromium.launch({headless:true});const page=await browser.newPage();await gateway.attach(page);
  await exercise(page,origin,repositoryId,repository,gateway.attach);
 } finally {
  await browser?.close();


 }
}

test('runtime preparation UI retains explicit grants and recovers exact commands', {timeout:120000}, ()=>withRepositoryUi(async(page,origin,repositoryId,repository,attachFixture)=>{
 await exerciseRuntimePreparation(page,origin,repositoryId,repository);
 await exerciseDefaultBranchRecheck(page,origin,repositoryId);
 await exerciseRepositoryOnboarding(page,origin,repositoryId,attachFixture);
}));

test('repository history pagination caches batches of ten and shows five at a time', {timeout:60000}, ()=>withRepositoryUi(exerciseRepositoryHistoryPagination));

test('delayed repository reads preserve the page structure and refresh keeps rows visible', {timeout:60000}, ()=>withRepositoryUi(async(page,origin,repositoryId)=>{
 await page.setViewportSize({width:724,height:853});
 let releaseCatalog,releaseHistory;
 const catalogGate=new Promise(resolve=>{releaseCatalog=resolve;});
 const historyGate=new Promise(resolve=>{releaseHistory=resolve;});
 const operation={id:randomUUID(),repositoryId,platform:'linux/arm64',state:'ready',stage:'cleanup',updatedAtUnixMs:String(Date.now()),contentVerified:true,managedCompatible:true,cleanupComplete:true};
 await page.route('**/api/ListRepositories',async route=>{await catalogGate;await route.fallback();});
 let historyHeld=true;
 await page.route('**/api/ListRuntimePreparations',async route=>{if(historyHeld)await historyGate;await route.fulfill({json:{operations:[operation]}});});
 await page.goto(`${origin}/repositories/${repositoryId}?tab=status`);
 const historyTitle=page.getByText(/^Previous preparations/);
 await historyTitle.waitFor();
 const tab=page.getByRole('tab',{name:'Status',exact:true});
 const beforeTab=await tab.boundingBox(),beforeHistory=await historyTitle.boundingBox();

 assert.equal(await page.getByRole('heading',{name:'Current preparation details',exact:true}).count(),0);
 assert.ok(await page.locator('[data-slot=skeleton]').count()>0);
 assert.equal(await page.getByRole('heading',{name:'Inspect a specific commit',exact:true}).count(),0);
 releaseCatalog();
 await page.getByRole('heading',{name:'fixture/runtime-ui',exact:true}).waitFor();
 const current=page.getByRole('region',{name:'Current preparation details',exact:true});
 await current.waitFor();
 assert.ok(await current.locator('[data-slot=skeleton]').count()>0);
 historyHeld=false;releaseHistory();
 const row=current.getByRole('article');
 await row.waitFor();
 const afterTab=await tab.boundingBox(),afterHistory=await historyTitle.boundingBox();
 assert.ok(Math.abs(beforeTab.y-afterTab.y)<2,`tabs moved ${beforeTab.y} → ${afterTab.y}`);
 assert.ok(Math.abs(beforeHistory.y-afterHistory.y)<2,`history moved ${beforeHistory.y} → ${afterHistory.y}`);
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
