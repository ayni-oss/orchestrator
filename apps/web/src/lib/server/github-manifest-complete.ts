import { error, redirect } from "@sveltejs/kit";
import type { ServerLoad } from "@sveltejs/kit";
import { consumeManifest } from "$lib/server/github-manifest.js";
import { getBff } from "$lib/server/bff.js";
import { requestContext, actionKey } from "$lib/server/page.js";
export const load: ServerLoad = async ({ cookies, locals, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store, max-age=0", "referrer-policy": "no-referrer" });
  const code = url.searchParams.get("code");
  const state = url.searchParams.get("state");
  const expected = cookies.get("github_app_manifest_state");
  cookies.delete("github_app_manifest_state", { path: "/setup/github" });
  cookies.delete("github_app_manifest_state", { path: "/" });
  if (!code || !state || !expected || state !== expected || !consumeManifest(state)) throw error(400, "GitHub App setup expired or could not be verified. Start again.");
  try { await getBff().completeGithubManifest(code, actionKey(), requestContext(locals)); }
  catch { throw error(502, "GitHub App setup could not be completed. Return to Source control to check the connection."); }
  redirect(303, "/source-control/github");
};
