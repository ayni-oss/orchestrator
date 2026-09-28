import { json, type RequestHandler } from "@sveltejs/kit";

export const GET: RequestHandler = () => json({ component: "web", status: "ready" }, {
  headers: { "cache-control": "no-store" },
});
