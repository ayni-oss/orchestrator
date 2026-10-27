import { gatewayOrigin, methods } from '$lib/server/boundary.js';
/** @type {import('./$types').RequestHandler} */
export async function POST({ params, request }) {
  if (!methods.has(params.method)) return new Response(null, { status: 404 });
  if (request.headers.get('x-console-request') !== '1' || !request.headers.get('content-type')?.startsWith('application/json')) return new Response(null, { status: 403 });
  const reader = request.body?.getReader();
  if (!reader) return new Response(null, { status: 400 });
  const chunks = []; let length = 0;
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    length += value.length;
    if (length > 65536) { await reader.cancel(); return new Response(null, { status: 413 }); }
    chunks.push(value);
  }
  const bytes = new Uint8Array(length); let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  const origin = gatewayOrigin();
  try {
    const response = await fetch(`${origin}/api/${params.method}`, {
      method: 'POST', redirect: 'error', signal: AbortSignal.timeout(25000),
      headers: { 'Content-Type': 'application/json', 'x-console-request': '1', Origin: origin }, body: bytes,
    });
    return new Response(response.body, { status: response.status, headers: { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' } });
  } catch {
    return Response.json({ message: 'Local service unavailable. Recover the operation before resubmitting credentials.' }, { status: 503 });
  }
}
