import {expect,it,vi} from "vitest";
import {eventStream} from "$lib/server/event-stream.js";
it("closes transport and cancels its producer when the request aborts",async()=>{
 const request=new AbortController();let signal:AbortSignal|undefined;
 const response=eventStream({signal:request.signal,requestId:"sse",label:"task",initialHeartbeat:true,run:async active=>{signal=active;await new Promise<void>(resolve=>active.addEventListener("abort",()=>resolve(),{once:true}));}});
 const reader=response.body!.getReader();await reader.read();await Promise.resolve();request.abort();expect(signal?.aborted).toBe(true);expect((await reader.read()).done).toBe(true);
});
it("bounds a slow consumer and cancels upstream delivery",async()=>{
 let signal:AbortSignal|undefined;
 const response=eventStream({signal:new AbortController().signal,requestId:"sse",label:"task",initialHeartbeat:true,run:async(active,emit)=>{signal=active;for(let i=0;i<50;i++)if(!emit(`data: ${i}\n\n`))break;}});
 await Promise.resolve();await Promise.resolve();expect(signal?.aborted).toBe(true);const text=await response.text();expect(text).toContain('"reason":"backpressure"');expect(text).not.toContain("data: 49");
});
it("does not start an upstream watch for an already-aborted request",async()=>{
 const request=new AbortController();request.abort();const run=vi.fn();const response=eventStream({signal:request.signal,requestId:"sse",label:"conversation",run});expect(await response.text()).toBe("");expect(run).not.toHaveBeenCalled();
});
