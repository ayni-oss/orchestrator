import {eventStream} from "$lib/server/event-stream.js";
import type { RequestHandler } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { isResourceVersion } from "$lib/server/validation.js";


export const GET: RequestHandler = ({ locals, params, request }) => {
  const scope = params.scope;
  const scopes = ["connections", "certificates", "projects", "runtimes", "repositories"] as const;
  if (scope !== "all" && !scopes.some(value => value === scope)) return new Response("invalid resource scope", { status: 404 });
  const cursor = request.headers.get("last-event-id") ?? undefined;
  if (cursor !== undefined && cursor !== "0" && !isResourceVersion(cursor)) {
    return new Response("the resource update cursor is invalid", { status: 400 });
  }
  // The combined stream resynchronizes each scope on reconnect. A single cursor
  // cannot represent five independently versioned resource streams.
  if (scope === "all" && cursor !== undefined) return new Response("combined resource streams do not accept a cursor", { status: 400 });

  return eventStream({signal:locals.requestSignal,requestId:locals.requestId,label:"resource",run:async(signal,emit)=>{
    let changedTimer:ReturnType<typeof setTimeout>|undefined;
    const pending=new Map<string,string>();
    const flush=()=>{changedTimer=undefined;for(const [changedScope,nextCursor] of pending){
      emit(scope==="all"?`event: changed\ndata: ${JSON.stringify({scope:changedScope})}\n\n`:`id: ${nextCursor}\nevent: changed\ndata: {}\n\n`);
    }pending.clear();};
    try{
      const selected=scope==="all"?scopes:scopes.filter(value=>value===scope);
      await Promise.all(selected.map(selectedScope=>getBff().watchResources(selectedScope,cursor,{requestId:locals.requestId,signal},nextCursor=>{
        pending.set(selectedScope,nextCursor);changedTimer??=setTimeout(flush,30);
      })));
    }finally{clearTimeout(changedTimer);if(!signal.aborted)flush();}
  }});
};
