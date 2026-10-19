import { connectionLoad, connectionActions } from "$lib/server/connections.js";
export const load = connectionLoad("github_app");
export const actions = connectionActions("github_app");
