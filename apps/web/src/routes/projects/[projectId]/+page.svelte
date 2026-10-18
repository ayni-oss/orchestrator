<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import type { PageProps } from "./$types";
  import { PanelRightOpen, MessageSquarePlus, MessagesSquare, Settings2, FileText } from "@lucide/svelte";
  import WorkspaceShell from "$lib/components/workspace-shell.svelte";
  import ApplicationNotifications from "$lib/components/application-notifications.svelte";
  import ProjectChat from "$lib/components/project-chat.svelte";
  import ProjectUpdates from "$lib/components/project-updates.svelte";
  import ProjectPlanReview from "$lib/components/project-plan-review.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Sheet from "$lib/components/ui/sheet/index.js";
  import * as Dropdown from "$lib/components/ui/dropdown-menu/index.js";
  import { projectHref } from "$lib/project-navigation.js";
  let { data }: PageProps = $props();
  const href: typeof projectHref = (...args) => projectHref(...args).replace("/_old/projects/", "/projects/");
  const planOpen = $derived(page.url.searchParams.get("view") === "plans");
  async function showPlan(open: boolean) {
    const url = new URL(page.url);
    if (open) url.searchParams.set("view", "plans");
    else { url.searchParams.delete("view"); url.searchParams.delete("version"); }
    await goto(url, { keepFocus: true, noScroll: true, replaceState: true });
  }
</script>

<svelte:head><title>{data.project?.displayName ?? "Project unavailable"} · Ayni Orchestrator</title></svelte:head>
<ApplicationNotifications />
<WorkspaceShell {data} title={data.project?.displayName ?? "Project"}>
  {#if data.project}
    <ProjectUpdates {data} />
    <div class="project-room">
      <header class="project-heading">
        <div><p class="eyebrow">Project</p><h1>{data.project.displayName}</h1><p class="goal">{data.project.goal}</p></div>
        <div class="project-tools">
          <Dropdown.Root><Dropdown.Trigger class="inline-flex size-9 items-center justify-center hover:bg-accent" aria-label="Conversation history"><MessagesSquare size={18} /></Dropdown.Trigger><Dropdown.Content align="end"><Dropdown.Label>Conversations</Dropdown.Label>{#each data.conversations as conversation (conversation.id)}<Dropdown.Item onclick={() => goto(href(data.project!.id, "overview", conversation.id))}>{conversation.title}</Dropdown.Item>{:else}<Dropdown.Item disabled>No conversations yet</Dropdown.Item>{/each}<Dropdown.Separator /><Dropdown.Item onclick={() => goto(`/projects/${encodeURIComponent(data.project!.id)}?new=${crypto.randomUUID()}`)}><MessageSquarePlus size={16} />New conversation</Dropdown.Item></Dropdown.Content></Dropdown.Root>
          <Button variant="ghost" size="icon" href={`/_old/projects/${encodeURIComponent(data.project.id)}?view=settings`} aria-label="Project settings"><Settings2 size={18} /></Button>
          <Button variant="outline" onclick={() => showPlan(true)}><PanelRightOpen size={16} />Plan{#if data.conversation?.plan}<span class="size-1.5 rounded-full bg-primary" aria-label="Plan available"></span>{/if}</Button>
        </div>
      </header>
      {#if data.configurationError}<Feedback kind="warning" title="Agent configuration unavailable" message={data.configurationError.message} />{/if}
      <div class="chat-column">
        {#key `${data.project.id}-${data.conversation?.id ?? page.url.searchParams.get("new") ?? ""}`}<ProjectChat {data} />{/key}
      </div>
    </div>
    <Sheet.Root open={planOpen} onOpenChange={showPlan}>
      <Sheet.Content side="right" class="data-[side=right]:w-full data-[side=right]:sm:max-w-xl">
        <Sheet.Header class="border-b p-6"><Sheet.Title>Project plan</Sheet.Title><Sheet.Description>Review scope and proposed work before approving execution.</Sheet.Description></Sheet.Header>
        <div class="min-h-0 flex-1 overflow-y-auto p-6">
          {#if data.conversationError}<Feedback kind="warning" title="Plan unavailable" message={data.conversationError.message} />
          {:else if data.conversation?.plan}{#key `${data.conversation.plan.id}-${data.planVersion}`}<ProjectPlanReview {data} {href} />{/key}
          {:else}<div class="flex min-h-72 flex-col items-center justify-center gap-4 text-center"><FileText class="size-9 text-primary" /><h2 class="text-lg font-medium">Your plan will take shape here</h2><p class="max-w-xs text-sm leading-relaxed text-muted-foreground">Start with the conversation. When the agent prepares a plan, you can review its scope, tasks, and acceptance criteria here.</p><Button variant="outline" onclick={() => showPlan(false)}>Back to chat</Button></div>{/if}
        </div>
      </Sheet.Content>
    </Sheet.Root>
  {:else}<Feedback kind="error" title="Project unavailable" message={data.error?.message ?? "This project could not be loaded."} />{/if}
</WorkspaceShell>

<style>
  .project-room{min-height:calc(100svh - 156px);display:flex;flex-direction:column;gap:25px}.project-heading{display:flex;align-items:flex-start;justify-content:space-between;gap:24px;padding-bottom:22px;border-bottom:1px solid var(--border)}.eyebrow{font:10px var(--font-mono);text-transform:uppercase;letter-spacing:1.5px;color:var(--muted-foreground);margin-bottom:10px}h1{font-size:27px;font-weight:600;letter-spacing:-.8px;overflow-wrap:anywhere}.goal{font-size:12px;line-height:1.7;color:var(--muted-foreground);margin-top:8px;max-width:620px}.project-tools{display:flex;gap:6px;align-items:center;flex-shrink:0}.chat-column{width:100%;max-width:780px;margin:0 auto;display:flex;min-height:450px;height:calc(100svh - 310px)}@media(max-width:700px){.project-heading{flex-direction:column;gap:16px}.project-tools{align-self:flex-end}.chat-column{height:auto;min-height:60svh}.project-room{min-height:calc(100svh - 125px)}}
</style>
