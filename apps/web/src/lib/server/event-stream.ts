import {BffError,type BrowserErrorDto} from "@ayni-orchestrator/bff/server";
const encoder=new TextEncoder();
export function streamError(error:unknown,requestId:string,label:string):BrowserErrorDto {
  return error instanceof BffError?error.toDto():{status:500,reason:"internal",message:`The ${label} update stream ended unexpectedly.`,requestId,retryable:false};
}
/** Shared bounded SSE lifecycle. Routes retain ownership of authorization and cursors. */
export function eventStream(options:{signal:AbortSignal;requestId:string;label:string;initialHeartbeat?:boolean;run:(signal:AbortSignal,emit:(payload:string)=>boolean)=>Promise<void>}):Response {
  const cancellation=new AbortController();let closed=false,heartbeat:ReturnType<typeof setInterval>|undefined;
  let finish=()=>{};
  const abort=()=>{cancellation.abort();finish();};
  const body=new ReadableStream<Uint8Array>({
    start(controller){
      finish=()=>{
        if(closed)return;closed=true;clearInterval(heartbeat);
        options.signal.removeEventListener("abort",abort);cancellation.abort();
        try{controller.close();}catch{/* Consumer has already disconnected. */}
      };
      const emit=(payload:string)=>{
        if(closed)return false;
        if((controller.desiredSize??1)<=0){
          const error=new BffError({status:503,reason:"backpressure",message:`The ${options.label} update consumer is too slow.`,requestId:options.requestId,retryable:true});
          controller.enqueue(encoder.encode(`event: error\ndata: ${JSON.stringify(error.toDto())}\n\n`));finish();return false;
        }
        controller.enqueue(encoder.encode(payload));return true;
      };
      if(options.signal.aborted){abort();return;}
      options.signal.addEventListener("abort",abort,{once:true});
      if(options.initialHeartbeat)emit("event: heartbeat\ndata: {}\n\n");
      heartbeat=setInterval(()=>emit("event: heartbeat\ndata: {}\n\n"),15000);
      void Promise.resolve().then(()=>{if(!cancellation.signal.aborted)return options.run(cancellation.signal,emit);}).catch(error=>{
        if(!cancellation.signal.aborted)emit(`event: error\ndata: ${JSON.stringify(streamError(error,options.requestId,options.label))}\n\n`);
      }).finally(()=>finish());
    },
    cancel(){closed=true;clearInterval(heartbeat);options.signal.removeEventListener("abort",abort);cancellation.abort();},
  },{highWaterMark:32,size:()=>1});
  return new Response(body,{headers:{"cache-control":"no-store","content-type":"text/event-stream; charset=utf-8","x-accel-buffering":"no"}});
}
