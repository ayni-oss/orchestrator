import { gatewayOrigin } from '$lib/server/boundary.js';
/** @type {import('./$types').RequestHandler} */
export async function GET({ params, url, request }) {
  if (!['stream', 'download'].includes(params.action)) return new Response(null, { status: 404 });
  const origin = gatewayOrigin();
  const headers = new Headers({ Origin: origin });
  const cursor = request.headers.get('last-event-id');
  if (cursor) headers.set('last-event-id', cursor);
  try {
    const response = await fetch(`${origin}/debug/logs/${params.action}${url.search}`, { headers, redirect: 'error', signal: request.signal });
    const output = new Headers({ 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' });
    for (const name of ['content-type', 'content-disposition']) { const value = response.headers.get(name); if (value) output.set(name, value); }
    return new Response(response.body, { status: response.status, headers: output });
  } catch { return new Response('Log stream unavailable', { status: 503, headers: { 'Cache-Control': 'no-store' } }); }
}
