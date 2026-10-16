import {json} from "@sveltejs/kit";
import type {RequestEvent} from "@sveltejs/kit";
import {getBff} from "$lib/server/bff.js";
import {requestContext,safeError} from "$lib/server/page.js";
import {isResourceVersion} from "$lib/server/validation.js";
const RESOURCE_ID=/^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;
export async function conversationHistory(event:RequestEvent){
  const {params,url,locals}=event;
  const headers={"cache-control":"no-store"};
  const projectId=params.projectId??"",conversationId=params.conversationId??"";
  const beforeSequence=url.searchParams.get("before")??"0",snapshotSequence=url.searchParams.get("snapshot")??"0",query=url.searchParams.get("q")??"";
  const limit=Number(url.searchParams.get("limit")??"50");
  if(!RESOURCE_ID.test(projectId)||!RESOURCE_ID.test(conversationId)||!isResourceVersion(beforeSequence)||!isResourceVersion(snapshotSequence)||!Number.isInteger(limit)||limit<1||limit>100||query.length>256){
    return json({error:{message:"Use valid conversation identifiers, cursors, a search of at most 256 characters, and a limit from 1 to 100."}},{status:400,headers});
  }
  try {
    const bff=getBff(),context=requestContext(locals),scope={projectId,conversationId};
    const result=await bff.listProjectConversationHistory({...scope,beforeSequence,snapshotSequence,query,limit},context);
    return json(result,{headers});
  }catch(error){const mapped=safeError(error,locals.requestId);return json({error:mapped},{status:mapped.status,headers});}
}

export type {ConversationHistoryDto,ConversationRecordDto} from "@ayni-orchestrator/bff/server";
