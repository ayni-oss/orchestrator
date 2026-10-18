import { json, type RequestHandler } from "@sveltejs/kit";
import { beginManifest } from "$lib/server/github-manifest.js";

const stateCookie = "github_app_manifest_state";

export const POST: RequestHandler = ({ cookies, request, url }) => {
  if (request.headers.get("origin") !== url.origin) return json({ error: "Invalid request origin." }, { status: 403, headers: { "cache-control": "no-store" } });
  cookies.delete(stateCookie, { path: "/_old/setup/github" });
  const state = beginManifest();
  cookies.set(stateCookie, state, {
    httpOnly: true,
    maxAge: 10 * 60,
    path: "/",
    sameSite: "lax",
    secure: process.env.NODE_ENV === "production",
  });
  return json({ state }, { headers: { "cache-control": "no-store" } });
};
