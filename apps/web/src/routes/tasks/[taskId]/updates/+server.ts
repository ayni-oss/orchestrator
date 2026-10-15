import type {RequestHandler} from "./$types";
import {getBff} from "$lib/server/bff.js";
import {isResourceVersion} from "$lib/server/validation.js";
import {eventStream} from "$lib/server/event-stream.js";
const RESOURCE_ID=/^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;
export const GET:RequestHandler=({locals,params,request})=>{
  if(!RESOURCE_ID.test(params.taskId))return new Response("a valid task identifier is required",{status:400});
  const cursor=request.headers.get("last-event-id")??undefined;
  if(cursor!==undefined&&!isResourceVersion(cursor))return new Response("the task update cursor is invalid",{status:400});
  return eventStream({signal:locals.requestSignal,requestId:locals.requestId,label:"task",run:async(signal,emit)=>{
    await getBff().watchDevelopmentTask(params.taskId,cursor,{requestId:locals.requestId,signal},update=>{
      emit(update.kind==="development_task"?`id: ${update.cursor}\nevent: task\ndata: ${JSON.stringify(update.developmentTask)}\n\n`:`id: ${update.cursor}\nevent: related\ndata: ${JSON.stringify(update)}\n\n`);
    });
  }});
};
