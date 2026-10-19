<script lang="ts">
  import type { PageProps } from './$types';
  import { invalidate } from '$app/navigation';
  import { Activity, TriangleAlert, GitPullRequest, Layers, ArrowUpRight, RefreshCw } from '@lucide/svelte';
  import WorkspaceShell from '$lib/components/workspace-shell.svelte';
  import PageHeader from '$lib/components/page-header.svelte';
  import Feedback from '$lib/components/feedback.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  let { data }: PageProps = $props();
  let refreshing = $state(false);
  const stages = ['Plan', 'Execute', 'Verify & deliver', 'Review'];
  const projectsUrl = $derived(data.workspace ? `/?workspace=${encodeURIComponent(data.workspace.id)}` : '/');
  const cards = $derived.by(() => {
    const projectName = (id:string) => data.projects.find(p => p.id === id)?.displayName ?? 'Project';
    const tasks = data.tasks.filter(t => t.state !== 'cancelled').map(task => ({
      id:task.id, title:task.snapshot.request, project:projectName(task.projectId),
      href:`/tasks/${encodeURIComponent(task.id)}`,
      stage:task.state === 'succeeded' ? 3 : ['queued','waiting_for_runtime','running'].includes(task.state) ? 1 : 2,
      attention:['failed','deadline_exceeded','unsupported','unknown'].includes(task.state),
      status:task.state.replaceAll('_',' '),
      detail:task.failure?.message ?? `${task.attemptCount} ${task.attemptCount === 1 ? 'attempt' : 'attempts'}`,
    }));
    const plans = data.conversations.filter(c => (c.plan?.versions.at(-1)?.state === 'pending' || c.proposals.some(p => p.state === 'pending')) || c.runs.some(r => ['queued','running'].includes(r.state))).map(c => ({
      id:c.id,title:c.title,project:projectName(c.projectId),href:`/projects/${encodeURIComponent(c.projectId)}?conversation=${encodeURIComponent(c.id)}&view=plans`,stage:0,
      attention:(c.plan?.versions.at(-1)?.state === 'pending' || c.proposals.some(p => p.state === 'pending')),status:(c.plan?.versions.at(-1)?.state === 'pending' || c.proposals.some(p => p.state === 'pending')) ? 'Awaiting approval' : 'Planning',detail:'Project conversation',
    }));
    return [...plans,...tasks];
  });
  const inProgress = $derived(cards.filter(c => !c.attention && c.stage < 3).length);
  const needsYou = $derived(cards.filter(c => c.attention).length);
  const review = $derived(cards.filter(c => c.stage === 3).length);
  async function refresh() { refreshing = true; try { await invalidate('app:operations'); } finally { refreshing = false; } }
</script>
<svelte:head><title>Operations · Ayni Orchestrator</title></svelte:head>
<WorkspaceShell {data} title="Operations">
  <PageHeader eyebrow="Workspace overview" title="Operations" description="Follow the work. See what needs you next." />
  {#if data.error}<Feedback kind="error" title="Operations unavailable" message={data.error.message}/><Button variant="outline" onclick={refresh} disabled={refreshing}>Retry</Button>
  {:else}
    <div class="metrics">
      <div><span>In progress <Activity size={18}/></span><strong>{inProgress.toString().padStart(2,'0')}</strong><small>Planning and execution</small></div>
      <div class:attention={needsYou > 0}><span>Needs you <TriangleAlert size={18}/></span><strong>{needsYou.toString().padStart(2,'0')}</strong><small>Decisions and blocked work</small></div>
      <div><span>Review <GitPullRequest size={18}/></span><strong>{review.toString().padStart(2,'0')}</strong><small>Completed tasks</small></div>
      <div><span>Active projects <Layers size={18}/></span><strong>{data.projects.filter(p => p.state === 'active').length.toString().padStart(2,'0')}</strong><small>In this workspace</small></div>
    </div>
    <div class="board-heading"><h2>Work in motion <small>{cards.length} {cards.length === 1 ? "item" : "items"}</small></h2><Button variant="ghost" size="icon" aria-label="Refresh operations" onclick={refresh} disabled={refreshing}><RefreshCw size={16} class={refreshing ? 'animate-spin' : ''}/></Button></div>
    {#if !cards.length}<div class="empty"><Layers size={28}/><h2>No work in motion yet</h2><p>Open a project and start a conversation to shape your first plan.</p><Button href={projectsUrl}>Open projects</Button></div>
    {:else}<div class="board">
      {#each stages as stage,index}<section class="column" aria-label={stage}><h3><span>0{index + 1} {stage}</span><span>{cards.filter(c => c.stage === index).length}</span></h3><div class="cards">{#each cards.filter(c => c.stage === index) as card (card.id)}<a class="work-card" class:attention={card.attention} href={card.href}><div class="card-top"><span>{card.project}</span><ArrowUpRight size={15}/></div><h4>{card.title}</h4><div class="card-bottom"><span>{card.status}</span>{#if card.attention}<TriangleAlert size={15}/>{/if}</div><p class="detail">{card.detail}</p></a>{:else}<p class="clear">All clear</p>{/each}</div></section>{/each}
    </div>{/if}
  {/if}
</WorkspaceShell>
<style>
h3,.clear,.metrics small,.card-bottom,.detail{font-family:var(--font-mono)}.metrics{display:grid;grid-template-columns:repeat(4,1fr);border:1px solid var(--border);background:var(--card);margin-bottom:34px}.metrics>div{padding:22px;border-right:1px solid var(--border)}.metrics>div:last-child{border-right:0}.metrics span{display:flex;justify-content:space-between;gap:8px;font-size:12px}.metrics strong{display:block;font-size:32px;font-weight:500;margin:12px 0}.metrics small{font-size:10px;color:var(--muted-foreground)}.attention{color:var(--primary)}.board-heading{display:flex;align-items:center;justify-content:space-between;gap:12px}.board-heading{margin-bottom:16px}h2{font-size:18px;font-weight:600}h2 small{font:10px var(--font-mono);margin-left:10px;color:var(--muted-foreground)}.board{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:18px;padding:22px;min-height:430px;border:1px solid var(--border);background-color:color-mix(in oklab,var(--primary) 3%,var(--background));background-image:radial-gradient(color-mix(in oklab,var(--primary) 20%,transparent) .6px,transparent .6px);background-size:20px 20px}h3{display:flex;justify-content:space-between;gap:8px;text-transform:uppercase;font-size:10px;letter-spacing:1px;color:var(--muted-foreground);margin-bottom:24px}.cards{display:grid;gap:16px}.work-card{display:block;min-width:0;padding:17px;background:var(--card);border:1px solid var(--border);box-shadow:3px 3px 0 var(--border);color:var(--foreground);transition:transform .15s}.work-card:hover{transform:translateY(-2px);border-color:var(--primary)}.work-card.attention{border-color:var(--primary)}.card-top{display:flex;justify-content:space-between;gap:8px;font-size:10px;color:var(--muted-foreground)}h4{font-size:14px;line-height:1.6;font-weight:500;margin:22px 0;display:-webkit-box;line-clamp:4;-webkit-line-clamp:4;-webkit-box-orient:vertical;overflow:hidden;overflow-wrap:anywhere}.card-bottom{display:flex;justify-content:space-between;border-top:1px solid var(--border);padding-top:12px;gap:8px;font-size:10px;text-transform:capitalize}.attention .card-bottom{color:var(--primary)}.detail{font-size:9px;line-height:1.6;color:var(--muted-foreground);margin-top:10px;overflow-wrap:anywhere;display:-webkit-box;line-clamp:2;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}.clear{text-align:center;font-size:10px;text-transform:uppercase;letter-spacing:1px;color:var(--muted-foreground);padding:35px 0}.empty{display:flex;flex-direction:column;align-items:center;gap:16px;padding:65px 20px;border:1px dashed var(--border);text-align:center}.empty p{color:var(--muted-foreground);font-size:13px}@media(max-width:1100px){.board{grid-template-columns:repeat(2,minmax(0,1fr))}.metrics{grid-template-columns:repeat(2,1fr)}.metrics>div{border-bottom:1px solid var(--border)}}@media(max-width:540px){.board{grid-template-columns:1fr}.metrics>div{padding:16px}.board{padding:16px}.board-heading h2 small{display:block;margin:6px 0 0}}
</style>
