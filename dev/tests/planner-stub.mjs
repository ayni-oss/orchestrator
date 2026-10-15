// Disposable local-only OpenAI Responses/model-list stub; never records credentials or prompts.
import {createServer} from "node:http";
import {pathToFileURL} from "node:url";
export function plannerStub(){
  let calls=0,completed=0,holding=false;
  const pending=new Set();
  function send(response,body,status=200){if(!response.destroyed){const value=JSON.stringify(body);response.writeHead(status,{"content-type":"application/json","content-length":Buffer.byteLength(value)});response.end(value);}}
  function complete(response,model,number){
    completed++;
    send(response,{id:`resp_fixture_${number}`,object:"response",created_at:1_760_000_000,status:"completed",model,error:null,incomplete_details:null,instructions:null,max_output_tokens:null,usage:{input_tokens:16,output_tokens:8,total_tokens:24},output:[{id:`msg_fixture_${number}`,type:"message",role:"assistant",status:"completed",content:[{type:"output_text",text:`Planner fixture completed turn ${number}. Original decisions remain retained.`}]}],tools:[]});
  }
  const server=createServer(async(request,response)=>{
    const path=new URL(request.url??"/","http://fixture").pathname;
    if(path==="/healthz")return send(response,{ready:true});
    if(path==="/fixture/stats")return send(response,{calls,completed,pending:pending.size,holding});
    if(request.method==="POST"&&path==="/fixture/hold"){holding=true;return send(response,{holding});}
    if(request.method==="POST"&&path==="/fixture/release"){
      holding=false;for(const item of pending){pending.delete(item);complete(item.response,item.model,item.number);}return send(response,{holding});
    }
    if(request.method==="GET"&&path==="/v1/models")return send(response,{object:"list",data:["gpt-5.6-sol","gpt-5.6-luna","fixture-model","session-fixture-model"].map(id=>({id,object:"model",created:1_760_000_000,owned_by:"fixture"}))});
    if(request.method!=="POST"||path!=="/v1/responses")return send(response,{error:{message:"unsupported fixture endpoint"}},404);
    try{
      let bytes=0;const chunks=[];
      for await(const chunk of request){bytes+=chunk.length;if(bytes>1_048_576)throw Error("request too large");chunks.push(chunk);}
      const value=JSON.parse(Buffer.concat(chunks).toString("utf8"));
      if(typeof value.model!=="string"||value.stream===true)throw Error("invalid fixture request");
      const number=++calls;
      if(holding){const item={response,model:value.model,number};pending.add(item);response.once("close",()=>pending.delete(item));}
      else complete(response,value.model,number);
    }catch{send(response,{error:{message:"invalid fixture request"}},400);}
  });
  return server;
}
if(process.argv[1]&&import.meta.url===pathToFileURL(process.argv[1]).href){
  plannerStub().listen(Number(process.env.PORT??8080),"0.0.0.0");
}
