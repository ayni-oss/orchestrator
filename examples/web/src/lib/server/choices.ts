/** Collect bounded choices without silently truncating or looping on repeated cursors. */
export async function collectChoices<T>(fetch:(pageToken:string)=>Promise<{items:T[];nextPageToken:string}>,limit=1000):Promise<T[]> {
  const values:T[]=[],seen=new Set<string>();let token="";
  do {const page=await fetch(token);values.push(...page.items);token=page.nextPageToken;
    if(values.length>limit||(token&&(seen.has(token)||values.length>=limit)))throw new Error("Choices could not be loaded completely.");
    seen.add(token);
  }while(token);
  return values;
}
