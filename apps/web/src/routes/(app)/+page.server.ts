import { redirect } from '@sveltejs/kit';
import { workspacePath, withCursor } from '$lib/paths';
import type { PageServerLoad } from './$types';
export const load: PageServerLoad = ({ url }) => {
 const workspace = url.searchParams.get('workspace');
 if (workspace) redirect(307, withCursor(workspacePath(workspace), 'afterProject', url.searchParams.get('afterProject') ?? ''));
};
