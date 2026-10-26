import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { defineConfig } from 'vite';
import { gatewayOrigin, localBoundary, readStatus, uiOrigin, uiPort } from './boundary.mjs';

const root = fileURLToPath(new URL('.', import.meta.url));
const namespace = process.env.AYNI_ORCHESTRATOR_NAMESPACE || 'ayni-orchestrator-live';
if (!/^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/.test(namespace) || namespace.length > 54) {
  throw new Error('Invalid live development namespace');
}
const statusPath = resolve(root, '../../.local/live', namespace, 'status.json');
const upstream = {
  target: gatewayOrigin,
  changeOrigin: true,
  configure(proxy) {
    proxy.on('proxyReq', (request) => {
      if (request.hasHeader('origin')) request.setHeader('origin', gatewayOrigin);
    });
  },
};

export default defineConfig({
  root,
  plugins: [{
    name: 'local-live-boundary',
    configureServer(server) {
      server.middlewares.use(localBoundary);
      server.middlewares.use('/__live/status', async (request, response) => {
        if (request.method !== 'GET' || request.url !== '/') {
          response.writeHead(404).end();
          return;
        }
        response.setHeader('Content-Type', 'application/json');
        response.end(JSON.stringify(await readStatus(statusPath)));
      });
    },
  }],
  server: {
    host: '127.0.0.1',
    port: uiPort,
    strictPort: true,
    cors: { origin: uiOrigin },
    fs: { strict: true, allow: [root] },
    proxy: { '/api': upstream, '/watch': upstream },
  },
});
