import { json, type RequestHandler } from "@sveltejs/kit";
import { beginManifest } from "$lib/server/github-manifest.js";

const stateCookie = "github_app_manifest_state";

export const POST: RequestHandler = ({ cookies }) => {
  const state = beginManifest();
  cookies.set(stateCookie, state, {
    httpOnly: true,
    maxAge: 10 * 60,
    path: "/setup/github",
    sameSite: "lax",
    secure: process.env.NODE_ENV === "production",
  });
  return json({ state }, { headers: { "cache-control": "no-store" } });
};
