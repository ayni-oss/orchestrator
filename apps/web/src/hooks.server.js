import { localRequest } from '$lib/server/boundary.js';
/** @type {import('@sveltejs/kit/hooks').Handle} */
export async function handle({ event, resolve }) {
  // Internal event.fetch requests use a framework-derived Origin, rather than
  // browser transport headers. isSubRequest is set by SvelteKit, never a header.
  const socket = event.isSubRequest ? undefined : event.platform?.req?.socket;
  if (!localRequest(event.url, event.request.headers, socket)) return new Response('Local operator origin required', { status: 403 });
  const response = await resolve(event);
  response.headers.set('Cache-Control', 'no-store');
  response.headers.set('Referrer-Policy', 'no-referrer');
  response.headers.set('X-Content-Type-Options', 'nosniff');
  response.headers.set('Content-Security-Policy', "frame-ancestors 'none'");
  return response;
}
