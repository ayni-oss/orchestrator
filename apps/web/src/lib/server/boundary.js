// Product remains a trusted loopback-only local operator surface.
/** @param {URL} url @param {Headers} headers @param {{localAddress?:string,localPort?:number,encrypted?:unknown}} [socket] */
export function localRequest(url, headers, socket) {
  if (url.hostname !== '127.0.0.1' || !['http:', 'https:'].includes(url.protocol)) return false;
  let origin = url.origin;
  if (socket) {
    if (socket.localAddress !== '127.0.0.1' || !Number.isInteger(socket.localPort) || !socket.localPort || socket.localPort < 1 || socket.localPort > 65535) return false;
    const requestPort = Number(url.port || (url.protocol === 'https:' ? 443 : 80));
    if (requestPort !== socket.localPort) return false;
    // Adapter-node 6 assumes HTTPS when reconstructing URLs. This direct local
    // server's transport determines its scheme; forwarded headers cannot do so.
    origin = new URL(`${socket.encrypted === true ? 'https' : 'http'}://127.0.0.1:${socket.localPort}`).origin;
  }
  const host = headers.get('host');
  if (socket && host === null) return false;
  if (host !== null && host !== new URL(origin).host) return false;
  return (headers.get('origin') === null || headers.get('origin') === origin)
    && [null, 'none', 'same-origin'].includes(headers.get('sec-fetch-site'));
}
export function gatewayOrigin(port = process.env.AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT || '3000') {
  if (!/^[0-9]+$/.test(port) || Number(port) < 1024 || Number(port) > 65535) throw new Error('Invalid local gateway port');
  return `http://127.0.0.1:${port}`;
}
export const methods = new Set(['SaveRuntimeSigner','SetRuntimeSignerEnabled','ListRuntimeSigners','GetRuntimeSignerReceipt','InspectRepository','GetRepositoryInspection','ListRepositoryInspections','RenameWorkspace','RenameProject','ListGithubInstallations','ListGithubRepositories','ConnectRepository','GetRepositoryOperation','ListRepositoryOperations','ListRepositories','AssociateProjectRepository','ListProjectRepositories','SaveGithubConnection', 'SaveProviderConnection', 'ValidateProviderConnection', 'SetConnectionEnabled', 'ValidateGithubConnection', 'GetConnectionOperation', 'ListConnections', 'ListConnectionOperations', 'ListWorkspaces', 'CreateWorkspace', 'ListProjects', 'CreateProject', 'GetAgentConfiguration', 'UpdateAgentConfiguration']);
