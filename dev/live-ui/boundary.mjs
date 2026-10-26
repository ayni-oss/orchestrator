import { readFile, stat } from 'node:fs/promises';

export function localPort(name, fallback) {
  const value = process.env[name] || String(fallback);
  if (!/^[0-9]+$/.test(value) || Number(value) < 1024 || Number(value) > 65535) {
    throw new Error(`${name} must be a port from 1024 to 65535`);
  }
  return Number(value);
}
export const uiPort = localPort('AYNI_ORCHESTRATOR_LIVE_UI_PORT', 5173);
export const gatewayPort = localPort('AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT', 3000);
if (uiPort === gatewayPort) throw new Error('Live UI and gateway ports must differ');
export const uiAuthority = `127.0.0.1:${uiPort}`;
export const uiOrigin = `http://${uiAuthority}`;
export const gatewayOrigin = `http://127.0.0.1:${gatewayPort}`;

// Check the original browser request before the proxy rewrites its upstream origin.
export function localRequest(headers) {
  return headers.host === uiAuthority
    && (headers.origin === undefined || headers.origin === uiOrigin)
    && (headers['sec-fetch-site'] === undefined
      || ['same-origin', 'none'].includes(headers['sec-fetch-site']));
}

export function localBoundary(request, response, next) {
  if (!localRequest(request.headers)) {
    response.writeHead(403).end('Local development origin required');
    return;
  }
  response.setHeader('Cache-Control', 'no-store');
  response.setHeader('X-Content-Type-Options', 'nosniff');
  response.setHeader('Referrer-Policy', 'no-referrer');
  next();
}

export async function readStatus(path, now = Date.now()) {
  try {
    const metadata = await stat(path);
    if (metadata.size > 64 * 1024) throw new Error('Status exceeds bound');
    const status = JSON.parse(await readFile(path, 'utf8'));
    return {
      status,
      observedAt: new Date(metadata.mtimeMs).toISOString(),
      stale: now - metadata.mtimeMs > 15_000,
    };
  } catch {
    return { status: null, stale: true, message: 'Live status unavailable; check dev/ctl live status.' };
  }
}
