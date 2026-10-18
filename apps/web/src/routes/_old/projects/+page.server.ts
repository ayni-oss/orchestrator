import {redirect} from "@sveltejs/kit";
import type {PageServerLoad} from "./$types";

export const load:PageServerLoad=({url})=>{
  const workspace=url.searchParams.get("workspace");
  redirect(303,workspace ? `/_old/workspaces/${encodeURIComponent(workspace)}` : "/_old/workspaces");
};
