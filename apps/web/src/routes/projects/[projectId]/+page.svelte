<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import type { PageProps } from "./$types";
  import { MessageSquarePlus, MessageSquare, Settings2 } from "@lucide/svelte";
  import WorkspaceShell from "$lib/components/workspace-shell.svelte";
  import ApplicationNotifications from "$lib/components/application-notifications.svelte";
  import ProjectChat from "$lib/components/project-chat.svelte";
  import ProjectUpdates from "$lib/components/project-updates.svelte";
  import ProjectPlans from "$lib/components/project-plans.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Tabs from "$lib/components/ui/tabs/index.js";
  import { projectHref } from "$lib/project-navigation.js";
  import { formatDate, statusTone } from "$lib/presentation.js";
  let { data }: PageProps = $props();
  const href = projectHref;
</script>

{#snippet conversationRail(view: "overview" | "plans")}
  <nav class="conversation-rail" aria-label="Project conversations">
    <div class="rail-heading"><h2>Conversations</h2><span>{data.conversations.length}</span></div>
    {#if view === "overview"}<button class:new-active={page.url.searchParams.has("new")} class="new-conversation" type="button" onclick={() => goto(`/projects/${encodeURIComponent(data.project!.id)}?new=${crypto.randomUUID()}`)}><MessageSquarePlus size={16} />New conversation</button>{/if}
    <div class="conversation-list">
      {#each data.conversations as conversation (conversation.id)}
        <a class:current={data.conversation?.id === conversation.id && !page.url.searchParams.has("new")} aria-current={data.conversation?.id === conversation.id && !page.url.searchParams.has("new") ? "page" : undefined} href={href(data.project!.id, view, conversation.id)}><MessageSquare size={15} /><span>{conversation.title}</span></a>
      {:else}<p>No conversations yet.</p>{/each}
    </div>
  </nav>
{/snippet}

<svelte:head><title>{data.project?.displayName ?? "Project unavailable"} · Ayni Orchestrator</title></svelte:head>
<ApplicationNotifications />
<WorkspaceShell {data} title={data.project?.displayName ?? "Project"}>
  {#if data.project}
    <ProjectUpdates {data} />
    <div class="project-room">
      <header class="project-heading">
        <div><p class="eyebrow">Project</p><h1>{data.project.displayName}</h1><p class="goal">{data.project.goal}</p></div>
        <div class="project-tools">
          <Button variant="ghost" size="icon" href={`/projects/${encodeURIComponent(data.project.id)}/settings`} aria-label="Project settings"><Settings2 size={18} /></Button>
        </div>
      </header>
      {#if data.configurationError}<Feedback kind="warning" title="Agent configuration unavailable" message={data.configurationError.message} />{/if}
      <Tabs.Root value={data.view} onValueChange={view => { if (view === "overview" || view === "plans" || view === "tasks") void goto(href(data.project!.id, view, data.conversation?.id), { keepFocus: true, noScroll: true }); }} class="min-h-0 flex-1 gap-5">
        <Tabs.List aria-label="Project views"><Tabs.Trigger value="overview">Chat</Tabs.Trigger><Tabs.Trigger value="plans">Plans</Tabs.Trigger><Tabs.Trigger value="tasks">Tasks</Tabs.Trigger></Tabs.List>
        <Tabs.Content value="overview" class="min-h-0 data-[state=active]:flex data-[state=active]:flex-col">
          {#if data.view === "overview"}<div class="project-content">{@render conversationRail("overview")}<div class="chat-column">{#key `${data.project.id}-${data.conversation?.id ?? page.url.searchParams.get("new") ?? ""}`}<ProjectChat {data} />{/key}</div></div>{/if}
        </Tabs.Content>
        <Tabs.Content value="plans" class="min-h-0 data-[state=active]:flex data-[state=active]:flex-col">
          {#if data.view === "plans"}<div class="project-content">{@render conversationRail("plans")}<div class="plans-column"><ProjectPlans {data} {href} /></div></div>{/if}
        </Tabs.Content>
        <Tabs.Content value="tasks">
          {#if data.view === "tasks"}<section class="space-y-4" aria-label="Project tasks"><div><h2 class="text-lg font-semibold">Tasks</h2><p class="text-sm text-muted-foreground">Work across this project’s repositories. Open a task for execution and quality details.</p></div><div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {#each data.tasks as task (task.id)}<a class="block border bg-card p-5 transition-colors hover:border-primary" href={`/tasks/${encodeURIComponent(task.id)}`}><div class="mb-4 flex items-start justify-between gap-3"><h3 class="font-medium leading-snug">{task.snapshot.request}</h3><StatusBadge status={task.state} tone={statusTone(task.state)} /></div><p class="text-xs text-muted-foreground">{data.repositories.find(repository => repository.id === task.snapshot.repository.repositoryConnectionId)?.displayName ?? "Removed repository · history retained"}</p><p class="mt-4 text-xs text-muted-foreground">{task.attemptCount} {task.attemptCount === 1 ? "attempt" : "attempts"} · Created {formatDate(task.createdAt)}</p></a>{:else}<p class="text-sm text-muted-foreground">No tasks yet. Approve a plan from a conversation to start work.</p>{/each}
          </div>{#if data.nextPageToken}<Button variant="outline" href={href(data.project.id, "tasks", data.conversation?.id, data.nextPageToken)}>More tasks</Button>{/if}</section>{/if}
        </Tabs.Content>
      </Tabs.Root>
    </div>
  {:else}<Feedback kind="error" title="Project unavailable" message={data.error?.message ?? "This project could not be loaded."} />{/if}
</WorkspaceShell>

<style>
  .project-room{min-height:calc(100svh - 156px);display:flex;flex-direction:column;gap:25px}.project-heading{display:flex;align-items:flex-start;justify-content:space-between;gap:24px;padding-bottom:22px;border-bottom:1px solid var(--border)}.eyebrow{font:10px var(--font-mono);text-transform:uppercase;letter-spacing:1.5px;color:var(--muted-foreground);margin-bottom:10px}h1{font-size:27px;font-weight:600;letter-spacing:-.8px;overflow-wrap:anywhere}.goal{font-size:12px;line-height:1.7;color:var(--muted-foreground);margin-top:8px;max-width:620px}.project-tools{display:flex;gap:6px;align-items:center;flex-shrink:0}.project-content{display:grid;grid-template-columns:220px minmax(0,1fr);gap:32px;width:100%}.conversation-rail{min-width:0;padding-top:4px;border-right:1px solid var(--border);padding-right:18px}.rail-heading{display:flex;align-items:center;justify-content:space-between;font-size:11px;font-weight:600;letter-spacing:.04em;text-transform:uppercase;color:var(--muted-foreground);margin-bottom:16px}.rail-heading span{font:11px var(--font-mono)}.new-conversation{display:flex;align-items:center;gap:9px;width:100%;text-align:left;cursor:pointer;padding:10px 9px;border:1px solid var(--border);font-size:12px;font-weight:600;margin-bottom:16px}.new-conversation:hover,.new-conversation.new-active{border-color:var(--primary);color:var(--primary)}.conversation-list{display:flex;flex-direction:column;gap:3px}.conversation-list a{display:flex;align-items:flex-start;gap:9px;min-width:0;padding:10px 9px;font-size:12px;line-height:1.4;color:var(--muted-foreground)}.conversation-list a span{overflow:hidden;text-overflow:ellipsis;display:-webkit-box;line-clamp:2;-webkit-line-clamp:2;-webkit-box-orient:vertical}.conversation-list a :global(svg){flex:none;margin-top:1px}.conversation-list a:hover{background:var(--accent);color:var(--foreground)}.conversation-list a.current{background:var(--accent);color:var(--foreground);font-weight:600;box-shadow:inset 2px 0 var(--primary)}.conversation-list p{font-size:12px;color:var(--muted-foreground);padding:8px}.chat-column{width:100%;max-width:780px;margin:0 auto;display:flex;min-height:450px;height:calc(100svh - 350px)}.plans-column{width:100%;max-width:880px;margin:0 auto;padding-bottom:50px}@media(max-width:900px){.project-content{grid-template-columns:180px minmax(0,1fr);gap:20px}}@media(max-width:700px){.project-heading{flex-direction:column;gap:16px}.project-tools{align-self:flex-end}.project-content{display:flex;flex-direction:column;gap:18px}.conversation-rail{border-right:0;border-bottom:1px solid var(--border);padding:0 0 14px}.conversation-list{max-height:170px;overflow-y:auto}.chat-column{height:auto;min-height:60svh}.project-room{min-height:calc(100svh - 125px)}}
</style>
