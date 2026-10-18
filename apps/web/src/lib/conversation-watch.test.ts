import {afterEach,expect,it,vi} from "vitest";
import {watchConversation} from "./conversation-watch.js";
class Source extends EventTarget {onopen:(()=>void)|undefined;close=vi.fn();}
afterEach(()=>vi.useRealTimers());
it("resynchronizes on connection and replaces expired cursors without leaking streams",async()=>{
 vi.useFakeTimers();const sources:Source[]=[],urls:string[]=[],changed=vi.fn();
 const stop=watchConversation({projectId:"p",conversationId:"c",cursor:"12"},changed,url=>{urls.push(url);const source=new Source();sources.push(source);return source as unknown as EventSource;});
 sources[0]!.onopen?.();expect(changed).toHaveBeenCalledTimes(1);
 sources[0]!.dispatchEvent(new MessageEvent("error",{data:JSON.stringify({reason:"watch_cursor_expired"})}));await vi.advanceTimersByTimeAsync(1000);expect(sources[0]!.close).toHaveBeenCalledOnce();expect(urls).toEqual(["/_old/projects/p/conversations/c/updates?after=12","/_old/projects/p/conversations/c/updates"]);
 stop();expect(sources[1]!.close).toHaveBeenCalledOnce();
});
