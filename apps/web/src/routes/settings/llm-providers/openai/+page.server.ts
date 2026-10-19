import { connectionLoad, connectionActions } from "$lib/server/connections.js";
export const load = connectionLoad("openai");
export const actions = connectionActions("openai");
