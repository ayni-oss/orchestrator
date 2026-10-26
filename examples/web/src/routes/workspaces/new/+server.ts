import { json, type RequestHandler } from '@sveltejs/kit';
import { getBff } from '$lib/server/bff.js';
import { requestContext, safeError } from '$lib/server/page.js';
import { projectRepositoryChoices } from '$lib/server/project-choices.js';
import { workspaceSchema } from '$lib/workspace-schema.js';

export const GET: RequestHandler = async ({ locals, setHeaders }) => {
  setHeaders({ 'cache-control': 'no-store' });
  try {
    const repositories = await projectRepositoryChoices(requestContext(locals));
    return json({ repositories: repositories.map(({ id, displayName }) => ({ id, displayName })) });
  } catch (error) {
    const mapped = safeError(error, locals.requestId);
    return json({ error: mapped.message }, { status: mapped.status });
  }
};

export const POST: RequestHandler = async ({ locals, request, url }) => {
  if (request.headers.get('origin') !== url.origin) return json({ error: 'Invalid request origin.' }, { status: 403 });
  let body: unknown;
  try { body = await request.json(); }
  catch { return json({ error: 'Enter a workspace name and choose repositories.' }, { status: 400 }); }
  const parsed = workspaceSchema.safeParse(body);
  if (!parsed.success) return json({ error: parsed.error.issues[0]?.message ?? 'Invalid workspace details.' }, { status: 400 });
  try {
    const workspace = await getBff().createWorkspace(parsed.data, requestContext(locals));
    return json({ id: workspace.id }, { status: 201, headers: { 'cache-control': 'no-store' } });
  } catch (error) {
    const mapped = safeError(error, locals.requestId);
    return json({ error: mapped.message }, { status: mapped.status });
  }
};
