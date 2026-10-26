import { randomBytes } from "node:crypto";
const states = new Map<string, number>();
const ttl = 10 * 60_000;
export function beginManifest(): string {
  const now=Date.now();
  for(const [state,expires] of states) if(expires<=now) states.delete(state);
  // Bound process memory; restarting intentionally expires pending setup sessions.
  if(states.size>=1000) throw new Error("Too many pending setup sessions");
  const state=randomBytes(32).toString("base64url");
  states.set(state,now+ttl);
  return state;
}
export function consumeManifest(state:string):boolean {
  const expires=states.get(state);
  states.delete(state);
  return expires!==undefined && expires>Date.now();
}
