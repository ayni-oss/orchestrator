import type { LayoutServerLoad } from './$types';
import { getBff } from '$lib/server/bff.js';
import { requestContext } from '$lib/server/page.js';
import { workspaceChoices, workspaceProjects } from '$lib/server/project-choices.js';
import { registryConfiguration } from '$lib/connection.js';

export const load: LayoutServerLoad = async ({ locals, depends, setHeaders }) => {
  depends('app:workspaces', 'app:projects', 'app:connections', 'app:certificates');
  setHeaders({ 'cache-control': 'no-store' });
  const context = requestContext(locals);
  const [workspaceResult, connectionResult, certificateResult] = await Promise.allSettled([
    workspaceChoices(context),
    getBff().listConnections(context),
    getBff().listCertificates(context),
  ]);
  const workspaces = workspaceResult.status === 'fulfilled' ? workspaceResult.value : [];
  const workspace = workspaces.find(item => item.state === 'active') ?? workspaces[0];
  const projects = workspace ? await workspaceProjects(workspace.id, context).catch(() => []) : [];
  const registryStatus = connectionResult.status === 'fulfilled'
    ? registryConfiguration(connectionResult.value)
    : { status: 'warning', description: 'Container registry permission status is unavailable.' };
  const certificateConfiguration = certificateResult.status === 'rejected'
    ? { status: 'warning', description: 'Certificate configuration status is unavailable.' }
    : certificateResult.value.length
      ? { status: 'ready', description: 'A signing certificate is configured.' }
      : { status: 'missing', description: 'No signing certificate is configured.' };
  return { workspaces, workspace, projects, registryConfiguration: registryStatus, certificateConfiguration };
};
