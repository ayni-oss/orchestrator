import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
export const load: PageServerLoad = async ({ parent }) => {
  const data = await parent();
  if (!data.debugEnabled) error(404, 'Debug logs are disabled for this deployment.');
  return {};
};
