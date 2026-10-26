import { connectionLoad, connectionActions } from "$lib/server/connections.js";
export const load = connectionLoad("ghcr");
export const actions = connectionActions("ghcr");
