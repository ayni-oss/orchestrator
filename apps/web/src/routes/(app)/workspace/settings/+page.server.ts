import { error, redirect } from '@sveltejs/kit';
import { workspaceSettingsPath } from '$lib/paths';
import type { PageServerLoad } from './$types';
export const load: PageServerLoad = ({ url }) => {
 const workspace = url.searchParams.get('workspace');
 if (!workspace) error(400, 'Choose a workspace before opening its settings.');
 redirect(307, workspaceSettingsPath(workspace));
};
