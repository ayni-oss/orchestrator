import { redirect } from "@sveltejs/kit";
export const load = () => redirect(303, "/_old/source-control/github");
