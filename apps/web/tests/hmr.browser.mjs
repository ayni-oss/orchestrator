import test from 'node:test';
import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createServer as createSocket } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { setTimeout as delay } from 'node:timers/promises';
import { chromium } from 'playwright';

const app=fileURLToPath(new URL('..',import.meta.url));
async function freePort(){const socket=createSocket();socket.listen(0,'127.0.0.1');await once(socket,'listening');const port=socket.address().port;await new Promise(resolve=>socket.close(resolve));return port;}

test('the application itself retains browser state through component and theme CSS HMR', {timeout:120000},async()=>{
 const directory=await realpath(await mkdtemp(join(tmpdir(),'ayni-web-hmr-')));
 const fixture=createServer((_request,response)=>{response.writeHead(200,{'Content-Type':'application/json'});response.end(JSON.stringify({workspaces:[]}));});
 let child,browser;
 try {
  fixture.listen(0,'127.0.0.1');await once(fixture,'listening');
  for(const name of ['src','static','package.json','svelte.config.js','vite.config.js','tsconfig.json'])await cp(join(app,name),join(directory,name),{recursive:true});
  await symlink(join(app,'node_modules'),join(directory,'node_modules'),'dir');
  const port=await freePort();const origin=`http://127.0.0.1:${port}`;
  child=spawn(resolve(app,'node_modules/.bin/vite'),['dev','--host','127.0.0.1','--port',String(port),'--strictPort'],{cwd:directory,stdio:'ignore',detached:true,env:{...process.env,AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT:String(fixture.address().port)}});
  let ready=false;for(let i=0;i<160;i++){try{if((await fetch(origin)).ok){ready=true;break;}}catch{}await delay(250);}
  assert.equal(ready,true,'application HMR server did not start');
  browser=await chromium.launch({headless:true});const page=await browser.newPage();
  await page.goto(origin);await page.waitForFunction(()=>document.documentElement.dataset.hydrated==='true');
  await page.evaluate(()=>{window.hmrSentinel='retained';});
  const source=join(directory,'src/routes/+page.svelte');
  const originalSource=await readFile(source,'utf8');
  assert.ok(originalSource.includes('title={label}'),'home page title binding must be found before testing HMR');
  await writeFile(source,originalSource.replace('title={label}','title="Updated projects"'));
  await page.getByRole('heading',{name:'Updated projects',exact:true}).waitFor();
  assert.equal(await page.evaluate(()=>window.hmrSentinel),'retained');
  const css=join(directory,'src/app.css');await writeFile(css,(await readFile(css,'utf8'))+'\n:root { --primary: rgb(12, 34, 56); }\n');
  await page.waitForFunction(()=>getComputedStyle(document.documentElement).getPropertyValue('--primary').trim()==='rgb(12, 34, 56)');
  assert.equal(await page.evaluate(()=>window.hmrSentinel),'retained');
 }finally{
  if(browser)await browser.close();
  if(child&&child.exitCode===null){process.kill(-child.pid,'SIGTERM');await once(child,'exit');}
  await new Promise(resolve=>fixture.close(resolve));await rm(directory,{recursive:true,force:true});
 }
});
