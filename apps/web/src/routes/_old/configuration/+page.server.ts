import { redirect } from "@sveltejs/kit";

export function load() {
  redirect(307, "/_old/source-control");
}
