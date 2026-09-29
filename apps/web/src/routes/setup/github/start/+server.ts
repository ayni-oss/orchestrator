import { json, type RequestHandler } from "@sveltejs/kit";
import { randomBytes } from "node:crypto";

const stateCookie = "github_app_manifest_state";

export const POST: RequestHandler = ({ cookies }) => {
  const state = randomBytes(32).toString("base64url");
  cookies.set(stateCookie, state, {
    httpOnly: true,
    maxAge: 10 * 60,
    path: "/setup/github",
    sameSite: "lax",
    secure: process.env.NODE_ENV === "production",
  });
  return json({ state }, { headers: { "cache-control": "no-store" } });
};
