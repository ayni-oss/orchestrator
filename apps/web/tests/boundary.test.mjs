import test from 'node:test';
import assert from 'node:assert/strict';
import { gatewayOrigin, localRequest, methods } from '../src/lib/server/boundary.js';
test('operator boundary rejects remote hosts and cross-origin requests', () => {
 const url = new URL('http://127.0.0.1:5174/settings/source-control/github');
 assert.equal(localRequest(url,new Headers()),true);
 assert.equal(localRequest(url,new Headers({origin:'http://evil.test'})),false);
 assert.equal(localRequest(url,new Headers({'sec-fetch-site':'cross-site'})),false);
 assert.equal(localRequest(new URL('http://remote.test'),new Headers()),false);
 assert.equal(localRequest(url,new Headers({origin:url.origin})),true);
});
test('gateway destination and RPC allowlist cannot be chosen by the browser',()=>{
 assert.equal(gatewayOrigin('3000'),'http://127.0.0.1:3000');
 assert.throws(()=>gatewayOrigin('3000@evil.test'));
 assert.throws(()=>gatewayOrigin('80'));
 assert.equal(methods.has('SaveGithubConnection'),true);
 assert.equal(methods.has('StartTaskPublication'),false);
});
test('adapter-node URL normalization uses the exact loopback transport without trusting forwarded headers',()=>{
 const url = new URL('https://127.0.0.1:5174/api/CreateWorkspace');
 const socket = {localAddress:'127.0.0.1',localPort:5174};
 const headers = new Headers({host:'127.0.0.1:5174',origin:'http://127.0.0.1:5174','sec-fetch-site':'same-origin'});
 assert.equal(localRequest(url,headers,socket),true);
 // Internal framework requests have their own URL-derived Origin and no Host.
 assert.equal(localRequest(url,new Headers({origin:url.origin})),true);
 assert.equal(localRequest(url,new Headers({origin:'https://evil.test'})),false);
 for (const values of [
  {origin:'http://evil.test'},
  {origin:'https://127.0.0.1:5174'},
  {origin:'http://127.0.0.1:5175'},
  {host:'evil.test:5174'},
  {host:'127.0.0.1:5175'},
  {host:'127.0.0.1:99999'},
  {'sec-fetch-site':'cross-site'},
  {origin:'https://evil.test','x-forwarded-proto':'https','x-forwarded-host':'evil.test'},
 ]) assert.equal(localRequest(url,new Headers({...Object.fromEntries(headers),...values}),socket),false);
 assert.equal(localRequest(url,new Headers({...Object.fromEntries(headers),'x-forwarded-proto':'https','x-forwarded-host':'evil.test'}),socket),true);
 assert.equal(localRequest(new URL('https://evil.test:5174'),headers,socket),false);
 assert.equal(localRequest(url,headers,{...socket,localPort:5175}),false);
 assert.equal(localRequest(url,headers,{...socket,localAddress:'192.0.2.1'}),false);
 assert.equal(localRequest(url,new Headers({origin:'http://127.0.0.1:5174'}),socket),false);
 const tlsHeaders = new Headers({host:'127.0.0.1:5174',origin:url.origin,'sec-fetch-site':'same-origin'});
 assert.equal(localRequest(url,tlsHeaders,{...socket,encrypted:true}),true);
});
