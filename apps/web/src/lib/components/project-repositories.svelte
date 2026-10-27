<script lang="ts">
 import { onMount } from 'svelte';
 import InformationCard from '$lib/components/information-card.svelte';
 import Feedback from '$lib/components/feedback.svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import * as NativeSelect from '$lib/components/ui/native-select/index.js';
 import { rpc, RpcError } from '$lib/api';
 import type { Repository } from '$lib/repositories';
 let {workspaceId,projectId}:{workspaceId:string;projectId:string}=$props();
 let catalog=$state.raw<Repository[]>([]);let attached=$state.raw<Repository[]>([]);let nextCatalog=$state('');let nextAttached=$state('');
 let repositoryId=$state('');let busy=$state(true);let error=$state('');let notice=$state('');
 type Association={idempotencyKey:string;workspaceId:string;projectId:string;repositoryId:string};
 let pending=$state<Association|null>(null);
 const storageKey=$derived(`ayni.repository.association.${workspaceId}.${projectId}`);
 function persist(value:Association|null){pending=value;if(value)sessionStorage.setItem(storageKey,JSON.stringify(value));else sessionStorage.removeItem(storageKey);}
 async function perform(action:()=>Promise<void>){busy=true;error='';try{await action();}catch(e){error=e instanceof RpcError&&e.status===409?'The selection conflicts with current configuration or a retained request. Refresh and review the selection; recover an uncertain request before starting another.':e instanceof RpcError&&e.status===404?'The selected resource is unavailable in this scope. Refresh the catalog and check the project and source connection.':e instanceof Error?e.message:'Project repositories could not be loaded.';}finally{busy=false;}}
 async function loadCatalog(append=false){const result=await rpc('ListRepositories',{afterId:append?nextCatalog:'',limit:50});catalog=append?[...catalog,...(result.repositories||[])]:result.repositories||[];nextCatalog=result.nextAfterId||'';}
 async function loadAttached(append=false){const result=await rpc('ListProjectRepositories',{workspaceId,projectId,afterId:append?nextAttached:'',limit:50});attached=append?[...attached,...(result.repositories||[])]:result.repositories||[];nextAttached=result.nextAfterId||'';}
 async function load(){await Promise.all([loadCatalog(),loadAttached()]);}
 async function recover(){if(!pending)return;if(pending.workspaceId!==workspaceId||pending.projectId!==projectId)throw new Error('The pending request belongs to a different project.');try{const result=await rpc('AssociateProjectRepository',pending);persist(null);notice=`${result.repository.fullName} attached to this project. Receipt ${result.commandId}.`;await loadAttached();}catch(e){if(e instanceof RpcError&&e.status===400)persist(null);throw e;}}
 async function associate(){if(!repositoryId||pending)return;persist({idempotencyKey:crypto.randomUUID(),workspaceId,projectId,repositoryId});await recover();}
 onMount(()=>{void perform(async()=>{const saved=sessionStorage.getItem(storageKey);if(saved)pending=JSON.parse(saved);await load();});});
</script>
<div class="space-y-6">
 {#if error}<Feedback kind="error" title="Project repositories need attention" message={error}/>{/if}
 {#if notice}<Feedback title="Project updated" message={notice}/>{/if}
 {#if pending}<InformationCard title="Recover project association"><p>The exact project and repository selection is retained. Retry to recover the same receipt.</p><div class="flex flex-wrap gap-3"><Button disabled={busy} onclick={()=>perform(recover)}>Recover association</Button><Button variant="outline" disabled={busy} onclick={()=>persist(null)}>Dismiss recovery</Button></div></InformationCard>{/if}
 <InformationCard title="Use a shared repository">
  <div class="space-y-2"><Label for="project-repository">Catalog repository</Label><NativeSelect.Root id="project-repository" class="w-full" bind:value={repositoryId} disabled={busy||!!pending}><NativeSelect.Option value="">Choose a repository</NativeSelect.Option>{#each catalog as repo (repo.id)}<NativeSelect.Option value={repo.id} disabled={attached.some(a=>a.id===repo.id)}>{repo.fullName}{attached.some(a=>a.id===repo.id)?' (attached)':''}</NativeSelect.Option>{/each}</NativeSelect.Root></div>
  <div class="flex flex-wrap gap-3"><Button disabled={busy||!!pending||!repositoryId||attached.some(a=>a.id===repositoryId)} onclick={()=>perform(associate)}>Attach repository</Button><Button variant="link" href="/repositories">Open shared catalog</Button></div>
  {#if nextCatalog}<Button variant="outline" disabled={busy} onclick={()=>perform(()=>loadCatalog(true))}>More catalog repositories</Button>{/if}
 </InformationCard>
 <div class="flex items-center justify-between"><h2 class="text-lg font-semibold">Project repositories</h2><Button variant="outline" disabled={busy} onclick={()=>perform(load)}>Refresh</Button></div>
 {#if !attached.length}<p class="text-sm text-muted-foreground">No repositories attached to this project.</p>{/if}
 {#each attached as repo (repo.id)}<InformationCard title={repo.fullName} status="connected"><p class="text-sm text-muted-foreground">{repo.defaultBranch} · {repo.private?'Private':'Public'}</p><p class="text-sm">Available to this project. Runtime preparation is still required before task execution.</p></InformationCard>{/each}
 {#if nextAttached}<Button variant="outline" disabled={busy} onclick={()=>perform(()=>loadAttached(true))}>More project repositories</Button>{/if}
</div>
