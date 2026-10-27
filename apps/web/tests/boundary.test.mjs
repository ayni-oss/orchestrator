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
