import { error, redirect } from '@sveltejs/kit';
import { projectSettingsPath } from '$lib/paths';
import type { PageServerLoad } from './$types';
export const load: PageServerLoad = ({ params, url }) => {
 const workspace = url.searchParams.get('workspace');
 if (!workspace) error(400, 'Choose a workspace before opening project settings.');
 redirect(307, projectSettingsPath(workspace, params.id));
};
