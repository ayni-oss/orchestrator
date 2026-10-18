import {BffError} from "@ayni-orchestrator/bff/server";
import {beforeEach,describe,expect,it,vi} from "vitest";
const bff=vi.hoisted(()=>({listProjectConversationHistory:vi.fn()}));
vi.mock("$lib/server/bff.js",()=>({getBff:()=>bff}));
import {GET as history} from "../routes/_old/projects/[projectId]/conversations/[conversationId]/history/+server.js";
function event(query="",projectId="project:1"){return {params:{projectId,conversationId:"conversation:1"},url:new URL(`http://localhost/history${query}`),locals:{requestId:"history-test",requestSignal:new AbortController().signal}} as never;}
describe("conversation history routes",()=>{
  beforeEach(()=>{vi.clearAllMocks();bff.listProjectConversationHistory.mockResolvedValue({records:[],snapshotSequence:"75",hasMore:false});});
  it("preserves explicit scope and frozen pagination without caching",async()=>{
    const response=await history(event("?before=50&snapshot=75&q=compatibility&limit=25"));
    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(bff.listProjectConversationHistory).toHaveBeenCalledWith({projectId:"project:1",conversationId:"conversation:1",beforeSequence:"50",snapshotSequence:"75",query:"compatibility",limit:25},expect.objectContaining({requestId:"history-test"}));
    expect(await response.json()).toMatchObject({records:[],hasMore:false});
  });
  it.each(["?before=-1","?snapshot=18446744073709551616","?limit=101","?limit=NaN",`?q=${"a".repeat(257)}`])("rejects unbounded input %s before backend access",async query=>{
    expect((await history(event(query))).status).toBe(400);expect(bff.listProjectConversationHistory).not.toHaveBeenCalled();
  });
  it("keeps archive failure distinct from an empty result",async()=>{
    bff.listProjectConversationHistory.mockRejectedValueOnce(new BffError({status:503,reason:"unavailable",message:"Archive unavailable",retryable:true}));
    const response=await history(event());expect(response.status).toBe(503);expect(await response.json()).toMatchObject({error:{message:"Archive unavailable"}});
  });
  it("preserves server rejection of another project's conversation",async()=>{
    bff.listProjectConversationHistory.mockRejectedValueOnce(new BffError({status:404,reason:"resource_not_found",message:"Conversation not found",retryable:false}));
    expect((await history(event("","project:other"))).status).toBe(404);
    expect(bff.listProjectConversationHistory).toHaveBeenCalledWith(expect.objectContaining({projectId:"project:other",conversationId:"conversation:1"}),expect.anything());
  });
});
