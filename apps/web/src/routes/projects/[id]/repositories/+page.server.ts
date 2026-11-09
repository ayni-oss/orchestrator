import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = ({ params, url }) => {
  redirect(307, `/projects/${params.id}/settings${url.search}`);
};
