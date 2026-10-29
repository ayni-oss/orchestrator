import { localRequest } from '$lib/server/boundary.js';
/** @type {import('@sveltejs/kit/hooks').Handle} */
export async function handle({ event, resolve }) {
  if (!localRequest(event.url, event.request.headers)) return new Response('Local operator origin required', { status: 403 });
  const response = await resolve(event);
  response.headers.set('Cache-Control', 'no-store');
  response.headers.set('Referrer-Policy', 'no-referrer');
  response.headers.set('X-Content-Type-Options', 'nosniff');
  response.headers.set('Content-Security-Policy', "frame-ancestors 'none'");
  return response;
}
