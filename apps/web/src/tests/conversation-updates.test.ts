import {beforeEach,describe,expect,it,vi} from "vitest";
const bff=vi.hoisted(()=>({
  getProjectConversation:vi.fn(async()=>({projectId:"project:1"})),
  watchProjectConversation:vi.fn(async(_id:string,_cursor:string|undefined,_ctx:{requestId:string;signal?:AbortSignal},changed:(cursor:string)=>void)=>{changed("9");}),
}));
vi.mock("$lib/server/bff.js",()=>({getBff:()=>bff}));
import {GET} from "../routes/_old/projects/[projectId]/conversations/[conversationId]/updates/+server.js";
function event(cursor="8",projectId="project:1"){
  const url=new URL("http://127.0.0.1/_old/projects/project:1/conversations/conversation:1/updates?after=7");
  return {params:{projectId,conversationId:"conversation:1"},url,locals:{requestId:"request:1",requestSignal:new AbortController().signal},request:new Request(url,{headers:{"last-event-id":cursor}})} as never;
}
describe("conversation update relay",()=>{
  beforeEach(()=>vi.clearAllMocks());
  it("resumes using Last-Event-ID and emits only an invalidation cursor",async()=>{
    const response=await GET(event());
    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(await response.text()).toBe("event: heartbeat\ndata: {}\n\nid: 9\nevent: changed\ndata: {}\n\n");
    expect(bff.watchProjectConversation).toHaveBeenCalledWith("conversation:1","8",expect.objectContaining({requestId:"request:1"}),expect.any(Function));
  });
  it("rejects a conversation from another project before opening a stream",async()=>{
    expect((await GET(event("8","project:other"))).status).toBe(404);
    expect(bff.watchProjectConversation).not.toHaveBeenCalled();
  });
  it.each(["invalid","18446744073709551616"])("rejects invalid cursor %s before gRPC",async cursor=>{
    expect((await GET(event(cursor))).status).toBe(400);
    expect(bff.getProjectConversation).not.toHaveBeenCalled();
  });
  it("cancels the upstream watch on browser disconnect",async()=>{
    bff.watchProjectConversation.mockImplementationOnce(async(_id,_cursor,ctx)=>{await new Promise<void>(resolve=>ctx.signal?.addEventListener("abort",()=>resolve(),{once:true}));});
    const response=await GET(event());await response.body?.cancel();
    expect(bff.watchProjectConversation.mock.calls[0]?.[2].signal?.aborted).toBe(true);
  });
});
