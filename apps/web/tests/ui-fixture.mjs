// Native browser request fixtures. They own no server, ports or infrastructure.
import { EventEmitter } from 'node:events';
export const origin = 'http://127.0.0.1:5174';
export function createFixture(handler) {
  const fixture = new EventEmitter();
  fixture.dispatch = async browserRequest => {
    const request = {
      url: new URL(browserRequest.url()).pathname,
      async *[Symbol.asyncIterator]() { yield browserRequest.postData() ?? ''; },
    };
    let status = 200, headers = {}, body = '';
    const response = {
      writeHead(code, values = {}) { status = code; headers = values; return response; },
      end(value = '') { body = value; return response; },
    };
    await handler(request, response);
    return { status, headers, body };
  };
  fixture.attach = page => page.context().route('**/api/**', async route => {
    await route.fulfill(await fixture.dispatch(route.request()));
  });
  return fixture;
}
