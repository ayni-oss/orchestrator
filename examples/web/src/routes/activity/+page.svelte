<script lang="ts">
  import type { PageProps } from './$types';
  import { invalidate } from '$app/navigation';
  import { Activity, GitBranch, Layers, MessageSquare, RefreshCw } from '@lucide/svelte';
  import WorkspaceShell from '$lib/components/workspace-shell.svelte';
  import PageHeader from '$lib/components/page-header.svelte';
  import Feedback from '$lib/components/feedback.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { formatDate } from '$lib/presentation.js';

  let { data }: PageProps = $props();
  let refreshing = $state(false);
  async function refresh() {
    refreshing = true;
    try { await invalidate('app:activity'); }
    finally { refreshing = false; }
  }
</script>

<svelte:head><title>Activity · Ayni Orchestrator</title></svelte:head>
<WorkspaceShell {data} title="Activity">
  <PageHeader eyebrow="Workspace / Recent work" title="Activity" description={`Recent conversations, plans, tasks, and repositories in ${data.workspace?.displayName ?? "this workspace"}.`}>
    {#snippet actions()}<Button variant="ghost" size="icon" aria-label="Refresh activity" title="Refresh activity" onclick={refresh} disabled={refreshing}><RefreshCw size={17} class={refreshing ? 'animate-spin' : ''}/></Button>{/snippet}
  </PageHeader>
  {#if data.error}<Feedback kind="error" title="Activity unavailable" message={data.error.message}/><Button variant="outline" onclick={refresh} disabled={refreshing}>Retry</Button>
  {:else if !data.workspace}<div class="empty"><Activity size={28}/><h2>No workspace selected</h2><p>Create or choose a workspace to see its work.</p></div>
  {:else}
    <section class="feed" aria-label="Recent workspace activity">
      <div class="feed-heading"><h2>Recent work</h2><span>{data.entries.length} {data.entries.length === 1 ? 'item' : 'items'}</span></div>
      {#each data.entries as entry (entry.id)}
        <a class="event" href={entry.href}>
          <span class="event-icon" aria-hidden="true">{#if entry.kind === 'task'}<GitBranch size={17}/>{:else if entry.kind === 'conversation'}<MessageSquare size={17}/>{:else if entry.kind === 'plan'}<Layers size={17}/>{:else}<Activity size={17}/>{/if}</span>
          <span class="event-content"><strong>{entry.title}</strong><span class="event-detail">{entry.detail}</span><small>{entry.context} · {entry.kind}</small></span>
          <time datetime={entry.at}>{formatDate(entry.at)}</time>
        </a>
      {:else}<div class="empty"><Activity size={28}/><h2>No recent work</h2><p>Conversations, plans, tasks, and repository updates will appear here.</p></div>{/each}
    </section>
    <p class="coverage">This view shows retained records and current task status. Workspace and project settings changes, and earlier task transitions, are not available here yet.</p>
  {/if}
</WorkspaceShell>

<style>
  .feed{border:1px solid var(--border);background:var(--card)}.feed-heading{display:flex;justify-content:space-between;align-items:center;padding:16px 20px;border-bottom:1px solid var(--border)}.feed-heading h2{font-size:13px;font-weight:600}.feed-heading span{font:10px var(--font-mono);color:var(--muted-foreground)}.event{display:flex;align-items:flex-start;gap:15px;padding:17px 20px;border-bottom:1px solid var(--border);color:var(--foreground)}.event:last-child{border-bottom:0}.event:hover{background:var(--accent)}.event-icon{display:grid;place-items:center;width:22px;height:22px;flex:none;color:var(--primary)}.event-content{display:flex;flex:1;min-width:0;flex-direction:column;gap:5px}.event-content strong{font-size:13px;font-weight:600}.event-detail{font-size:12px;line-height:1.5;white-space:pre-line;overflow:hidden;display:-webkit-box;line-clamp:2;line-clamp:2;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow-wrap:anywhere}.event-content small{font:10px var(--font-mono);color:var(--muted-foreground);text-transform:capitalize}.event time{flex:none;font:10px var(--font-mono);color:var(--muted-foreground);margin-top:3px}.coverage{font-size:11px;line-height:1.6;color:var(--muted-foreground);margin-top:16px}.empty{display:flex;flex-direction:column;align-items:center;gap:12px;padding:64px 20px;color:var(--muted-foreground);text-align:center}.empty h2{font-size:16px;color:var(--foreground)}.empty p{font-size:12px}@media(max-width:700px){.event{padding:14px;gap:10px}.event time{max-width:100px;text-align:right;line-height:1.5}}
</style>
