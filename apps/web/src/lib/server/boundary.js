// Product remains a trusted loopback-only local operator surface.
/** @param {URL} url @param {Headers} headers */
export function localRequest(url, headers) {
  return url.hostname === '127.0.0.1'
    && (headers.get('origin') === null || headers.get('origin') === url.origin)
    && [null, 'none', 'same-origin'].includes(headers.get('sec-fetch-site'));
}
export function gatewayOrigin(port = process.env.AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT || '3000') {
  if (!/^[0-9]+$/.test(port) || Number(port) < 1024 || Number(port) > 65535) throw new Error('Invalid local gateway port');
  return `http://127.0.0.1:${port}`;
}
export const methods = new Set(['SaveGithubConnection', 'SetConnectionEnabled', 'ValidateGithubConnection', 'GetConnectionOperation', 'ListConnections', 'ListConnectionOperations', 'ListWorkspaces', 'CreateWorkspace', 'ListProjects', 'CreateProject']);
