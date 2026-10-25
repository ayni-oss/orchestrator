// Browser-boundary fixture only. Deployed acceptance uses WEB_URL instead.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve, extname } from 'node:path';
import { randomUUID } from 'node:crypto';
const build = fileURLToPath(new URL('../build/', import.meta.url));
export async function startFixture() {
  const workspaces = [], projects = [], conversations = [], snapshots = new Map(), receipts = new Map(), watchers = new Map();
  const timestamp = () => new Date().toISOString();
  function requireScope(body) {
    const conversation = conversations.find(value => value.id === body.conversationId && value.workspaceId === body.workspaceId && value.projectId === body.projectId);
    if (!conversation) throw { code: 'NotFound', reason: 'ERROR_REASON_RESOURCE_NOT_FOUND' };
    return snapshots.get(conversation.id);
  }
  function frame(value) { return `id: ${value.snapshot?.resumeCursor || value.update?.cursor}\nevent: frame\ndata: ${JSON.stringify(value)}\n\n`; }
  function notify(snapshot) {
    for (const response of watchers.get(snapshot.conversationId) || []) response.write(frame({ update: { cursor: snapshot.resumeCursor, conversationRevision: snapshot.conversationRevision, safeReason: snapshot.currentActivity.safeReason, presentationState: snapshot.currentActivity.presentationState, acceptedAt: timestamp() } }));
  }
  function update(snapshot, reason, terminal) {
    snapshot.conversationRevision = String(BigInt(snapshot.conversationRevision) + 1n);
    snapshot.resumeCursor = `${snapshot.conversationId}-${snapshot.conversationRevision}`;
    snapshot.currentActivity = { stage: terminal ? 'ACTIVITY_STAGE_TERMINAL' : 'ACTIVITY_STAGE_EXECUTION', presentationState: terminal ? 'PRESENTATION_STATE_COMPLETED' : 'PRESENTATION_STATE_PROCESSING', safeReason: reason, acceptedAt: timestamp() };
    snapshot.historyCounts = { messages: String(snapshot.recentMessages.length), turns: String(Math.ceil(snapshot.recentMessages.length / 2)), hasOlderHistory: snapshot.recentMessages.length > 16 };
    notify(snapshot);
  }
  async function api(method, body) {
    if (receipts.has(body.idempotencyKey)) return receipts.get(body.idempotencyKey);
    let result;
    const id = randomUUID();
    const receipt = { commandId: id };
    if (method.startsWith('Create') && !(body.displayName || body.title)?.trim()) throw { code: 'InvalidArgument', reason: 'ERROR_REASON_INVALID_INPUT' };
    switch (method) {
      case 'ListWorkspaces': return { workspaces };
      case 'ListProjects': return { projects: projects.filter(value => value.workspaceId === body.workspaceId) };
      case 'ListConversations': return { conversations: conversations.filter(value => value.workspaceId === body.workspaceId && value.projectId === body.projectId) };
      case 'CreateWorkspace': { const workspace = { id, displayName: body.displayName }; workspaces.push(workspace); result = { workspace, receipt }; break; }
      case 'CreateProject': { const project = { id, workspaceId: body.workspaceId, displayName: body.displayName, goal: body.goal }; projects.push(project); result = { project, receipt }; break; }
      case 'CreateConversation': {
        const conversation = { id, workspaceId: body.workspaceId, projectId: body.projectId, title: body.title, createdAt: timestamp() }; conversations.push(conversation);
        snapshots.set(id, { ...body, conversationId: id, conversationRevision: '1', title: body.title, recentMessages: [], nonterminalTurns: [], recentTerminalTurns: [], historyCounts: { messages: '0', turns: '0' }, resumeCursor: `${id}-1`, historySnapshotId: `history-${id}` });
        result = { conversation, receipt }; break;
      }
      case 'GetConversationSnapshot': { const snapshot = requireScope(body); return { snapshot: { ...snapshot, recentMessages: snapshot.recentMessages.slice(-16) } }; }
      case 'ListProviderCalls': requireScope(body); return { calls: [] };
      case 'ListConversationHistory': {
        const snapshot = requireScope(body);
        return { records: snapshot.recentMessages.map(message => ({ id: message.id, sequence: message.sequence, kind: 'CONVERSATION_HISTORY_RECORD_KIND_MESSAGE', roleOrState: message.role === 'MESSAGE_ROLE_USER' ? 'CONVERSATION_HISTORY_ROLE_OR_STATE_USER_MESSAGE' : 'CONVERSATION_HISTORY_ROLE_OR_STATE_ASSISTANT_MESSAGE', contentOrOutcome: message.content, acceptedAt: message.acceptedAt })).reverse(), hasMore: false, snapshotId: snapshot.historySnapshotId };
      }
      case 'SendMessage': {
        const snapshot = requireScope(body);
        snapshot.recentMessages.push({ id, content: body.content, sequence: String(snapshot.recentMessages.length + 1), role: 'MESSAGE_ROLE_USER', acceptedAt: timestamp() });
        snapshot.nonterminalTurns = [{ id, acceptedAt: timestamp(), executionState: 'EXECUTION_STATE_PROCESSING' }];
        update(snapshot, 'fixture_execution', false);
        const timer = setTimeout(() => {
          if (!snapshot.nonterminalTurns.length) return;
          snapshot.recentMessages.push({ id: randomUUID(), content: `Scripted response: ${body.content}`, sequence: String(snapshot.recentMessages.length + 1), role: 'MESSAGE_ROLE_ASSISTANT', acceptedAt: timestamp() });
          snapshot.nonterminalTurns = []; snapshot.recentTerminalTurns = [{ id, presentationState: 'PRESENTATION_STATE_COMPLETED' }];
          update(snapshot, 'terminal_outcome_accepted', true);
        }, 150); timer.unref();
        result = { receipt: { ...receipt, turnId: id } }; break;
      }
      case 'CancelTurn': { const snapshot = requireScope(body); snapshot.nonterminalTurns = []; update(snapshot, 'cancelled', true); result = { commandId: id }; break; }
      default: throw { code: 'Unimplemented', reason: method };
    }
    if (body.idempotencyKey) receipts.set(body.idempotencyKey, result);
    return result;
  }
  const sockets = new Set();
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1');
    response.setHeader('Cache-Control', 'no-store');
    response.setHeader('Content-Security-Policy', "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'");
    try {
      if (url.pathname.startsWith('/api/')) {
        let body = ''; for await (const part of request) body += part;
        const result = await api(url.pathname.slice(5), JSON.parse(body));
        response.setHeader('Content-Type', 'application/json'); response.end(JSON.stringify(result)); return;
      }
      if (url.pathname === '/watch') {
        const snapshot = requireScope(Object.fromEntries(url.searchParams));
        response.setHeader('Content-Type', 'text/event-stream');
        response.write(frame({ snapshot: { ...snapshot, recentMessages: snapshot.recentMessages.slice(-16) } }));
        const set = watchers.get(snapshot.conversationId) || new Set(); set.add(response); watchers.set(snapshot.conversationId, set);
        request.on('close', () => set.delete(response)); return;
      }
      const path = resolve(build, url.pathname === '/workspace/' ? 'index.html' : `.${url.pathname.replace('/workspace', '')}`);
      if (!path.startsWith(build) || !url.pathname.startsWith('/workspace/')) { response.writeHead(404).end(); return; }
      const body = await readFile(path);
      response.setHeader('Content-Type', { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' }[extname(path)] || 'application/octet-stream'); response.end(body);
    } catch (error) { response.writeHead(error.code === 'NotFound' ? 404 : 400, { 'Content-Type': 'application/json' }).end(JSON.stringify({ code: error.code || 'Internal', reason: error.reason || 'fixture_failure' })); }
  });
  server.on('connection', socket => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return { url: `http://127.0.0.1:${server.address().port}/workspace/`, async close() { for (const socket of sockets) socket.destroy(); await new Promise(resolve => server.close(resolve)); } };
}
