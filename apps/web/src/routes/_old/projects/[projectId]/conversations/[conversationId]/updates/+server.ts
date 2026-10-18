import {eventStream,streamError} from "$lib/server/event-stream.js";
import type { RequestHandler } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { isResourceVersion } from "$lib/server/validation.js";

const RESOURCE_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;

export const GET: RequestHandler = async ({ locals, params, request, url }) => {
  if (!RESOURCE_ID.test(params.conversationId)) {
    return new Response("a valid conversation identifier is required", { status: 400 });
  }
  const cursor = request.headers.get("last-event-id") ?? url.searchParams.get("after") ?? undefined;
  if (cursor !== undefined && !isResourceVersion(cursor)) {
    return new Response("the conversation update cursor is invalid", { status: 400 });
  }

  try {
    const conversation=await getBff().getProjectConversation(params.conversationId,{requestId:locals.requestId,signal:locals.requestSignal});
    if(conversation.projectId!==params.projectId)return new Response("Conversation does not belong to this project",{status:404});
  }catch(error){const mapped=streamError(error,locals.requestId,"conversation");return Response.json({error:mapped},{status:mapped.status});}
  return eventStream({signal:locals.requestSignal,requestId:locals.requestId,label:"conversation",initialHeartbeat:true,run:async(signal,emit)=>{
    await getBff().watchProjectConversation(params.conversationId,cursor,{requestId:locals.requestId,signal},updateCursor=>{
      emit(`id: ${updateCursor}\nevent: changed\ndata: {}\n\n`);
    });
  }});
};
