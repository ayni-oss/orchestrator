/** One selected conversation stream, with explicit recovery after cursor expiry. */
export function watchConversation(selection:{projectId:string;conversationId:string;cursor:string},changed:()=>void,open:(url:string)=>EventSource=url=>new EventSource(url)){
  let source:EventSource|undefined,retry:ReturnType<typeof setTimeout>|undefined,stopped=false,cursor:string|undefined=selection.cursor;
  function connect(){
    if(stopped)return;
    const url=`/_old/projects/${encodeURIComponent(selection.projectId)}/conversations/${encodeURIComponent(selection.conversationId)}/updates${cursor===undefined?"":`?after=${encodeURIComponent(cursor)}`}`;
    const stream=source=open(url);
    stream.onopen=()=>{changed();};
    stream.addEventListener("changed",event=>{const id=(event as MessageEvent).lastEventId;if(id)cursor=id;changed();});
    stream.addEventListener("error",event=>{
      if(event instanceof MessageEvent){try{
        if((JSON.parse(String(event.data)) as {reason?:string}).reason==="watch_cursor_expired"){
          stream.close();cursor=undefined;changed();retry=setTimeout(connect,1000);
        }
      }catch{/* Native errors reconnect with Last-Event-ID. */}}
    });
  }
  connect();
  return ()=>{stopped=true;clearTimeout(retry);source?.close();};
}
