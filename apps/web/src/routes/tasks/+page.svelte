<script lang="ts">
  import { page } from "$app/state";
  import { enhance } from "$app/forms";
  import { Play } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import CodeValue from "$lib/components/code-value.svelte";
  import EmptyState from "$lib/components/empty-state.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import SubmitButton from "$lib/components/submit-button.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import * as Table from "$lib/components/ui/table/index.js";
  import { Textarea } from "$lib/components/ui/textarea/index.js";
  import { formatDate, statusTone, words } from "$lib/presentation.js";

  let { data, form }: PageProps = $props();
  let submitting = $state(false);
  let projectChoice=$state<string | undefined>();
  const selectedProject=$derived(projectChoice ?? page.url.searchParams.get("project") ?? data.projects.find(p=>p.repositoryConnectionIds.includes(page.url.searchParams.get("repository") ?? ""))?.id ?? "");
  let repositoryChoice = $state("");
  const members = $derived(data.projects.find(p=>p.id===selectedProject)?.repositoryConnectionIds ?? []);
  const selectedRepository = $derived(members.includes(repositoryChoice) ? repositoryChoice : members.length === 1 ? members[0]! : "");
  let selectedProfile=$state(page.url.searchParams.get("profile") ?? "");
  const repositoryProfiles=$derived(data.runtimeProfiles.filter(p=>p.repositoryConnectionId===selectedRepository));
  const readyProfiles = $derived(repositoryProfiles.filter((profile) => profile.state === "ready" && profile.support === "operational"));
  const nextPageHref = $derived(data.nextPageToken === undefined ? undefined : `/tasks?pageToken=${encodeURIComponent(data.nextPageToken)}`);
</script>

<svelte:head><title>Development tasks · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8" data-bff-status={data.backend}>
  <PageHeader eyebrow="Execution" title="Development tasks" description="Each task freezes repository revision, profile revision, runtime digest, agent/model selection, request, constraints, and deadline." />
  {#if data.error}<Feedback kind="warning" title="Some task data is unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if form?.error}<Feedback kind="error" title="Task submission failed" message={form.error.message} requestId={form.error.requestId} />{/if}

  <div class="grid items-start gap-6 xl:grid-cols-[minmax(0,1fr)_24rem]">
    <section class="space-y-4"><div><h2 class="text-lg font-semibold">Task history</h2><p class="text-sm text-muted-foreground">Completion does not imply publication or assessment readiness.</p></div>
      {#if data.tasks.length === 0}<EmptyState title="No tasks submitted" description="Submit bounded work against a ready runtime profile." />{:else}
        <div class="overflow-x-auto rounded-lg border bg-card"><Table.Root><Table.Header><Table.Row><Table.Head>Request</Table.Head><Table.Head>Status</Table.Head><Table.Head>Repository revision</Table.Head><Table.Head>Runtime digest</Table.Head><Table.Head>Updated</Table.Head></Table.Row></Table.Header><Table.Body>{#each data.tasks as task (task.id)}<Table.Row><Table.Cell><a class="font-medium hover:underline" href={`/tasks/${encodeURIComponent(task.id)}`}>{task.snapshot.request}</a><span class="block text-xs text-muted-foreground">{task.snapshot.agent.model}</span></Table.Cell><Table.Cell><StatusBadge status={task.state} tone={statusTone(task.state)} /></Table.Cell><Table.Cell><CodeValue value={task.snapshot.repository.revision.slice(0, 12)} label={task.snapshot.repository.revision} /></Table.Cell><Table.Cell><CodeValue value={task.snapshot.runtime.resolvedDigest.slice(0, 19)} label={task.snapshot.runtime.resolvedDigest} /></Table.Cell><Table.Cell>{formatDate(task.updatedAt)}</Table.Cell></Table.Row>{/each}</Table.Body></Table.Root></div>
        {#if nextPageHref}<div class="flex justify-end"><Button href={nextPageHref} variant="outline">Next page</Button></div>{/if}
      {/if}
    </section>

    <Card.Root id="new" class="xl:sticky xl:top-24"><Card.Header><Card.Title>Submit task</Card.Title><Card.Description>Only operational, ready profiles can admit work.</Card.Description></Card.Header><Card.Content>
      {#if readyProfiles.length === 0 || data.models.length === 0}<Feedback kind="warning" title="Submission unavailable" message={readyProfiles.length === 0 ? "No runtime profile is ready." : "No provider model is available."} />{/if}
      <form method="POST" action="?/create" class="mt-4 space-y-4" use:enhance={() => { submitting = true; return async ({ update }) => { submitting = false; await update(); }; }}>
        <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="provider" value={data.provider ?? ""} />
        <div class="space-y-2"><Label for="project">Project</Label><select id="project" name="projectId" value={selectedProject} onchange={(event)=>{projectChoice=event.currentTarget.value;repositoryChoice="";selectedProfile="";}} required class="h-9 w-full rounded-lg border bg-background px-3 text-sm"><option value="" disabled>Select a project</option>{#each data.projects as item (item.id)}<option value={item.id}>{item.displayName}</option>{/each}</select></div>
        <div class="space-y-2"><Label for="repository">Repository</Label><select id="repository" name="repositoryConnectionId" value={selectedRepository} onchange={event => {repositoryChoice=event.currentTarget.value;selectedProfile="";}} required class="h-9 w-full rounded-lg border bg-background px-3 text-sm"><option value="" disabled>Select a repository</option>{#each members as id (id)}<option value={id}>{id}</option>{/each}</select></div>
        <div class="space-y-2"><Label for="profile">Runtime profile</Label><select id="profile" name="runtimeProfileId" bind:value={selectedProfile} required disabled={readyProfiles.length === 0} class="h-9 w-full rounded-lg border bg-background px-3 text-sm"><option value="" disabled>Select a ready profile</option>{#each readyProfiles as profile (profile.id)}<option value={profile.id}>{words(profile.kind)}</option>{/each}</select></div>
        <div class="space-y-2"><Label for="model">Model</Label><select id="model" name="model" required class="h-9 w-full rounded-lg border bg-background px-3 text-sm"><option value="" disabled>Select a model</option>{#each data.models as model (model.id)}<option value={model.id}>{model.name ?? model.id}</option>{/each}</select></div>
        <p class="text-xs text-muted-foreground">The selected profile’s verified repository revision and image digest are frozen at submission. Later builds do not change this request.</p>
        {#each repositoryProfiles as profile (profile.id)}{#if profile.state!=="ready"}<p class="text-sm"><a class="underline" href={`/profiles/${encodeURIComponent(profile.id)}`}>{words(profile.kind)}: {words(profile.state)} — view next step</a></p>{/if}{/each}
        <p class="text-sm text-muted-foreground">Work is saved and pushed to a draft PR automatically, including when checks fail or the agent stops early.</p>
        <div class="space-y-2"><Label for="request">Request</Label><Textarea id="request" name="request" required maxlength={16384} rows={4} /></div>
        <div class="space-y-2"><Label for="criteria">Acceptance criteria</Label><Textarea id="criteria" name="acceptanceCriteria" required rows={3} placeholder="One criterion per line" /></div>
        <details class="rounded-lg border p-3"><summary>Constraints and deadline</summary><div class="mt-3 space-y-3"><Textarea name="protectedPaths" rows={2} placeholder="Protected paths, one per line" /><label class="flex gap-2 text-sm"><input type="checkbox" name="allowTestEdits" /> Allow test edits</label><Input name="deadlineSeconds" type="number" min={1} max={86400} placeholder="Deadline seconds" /></div></details>
        <SubmitButton class="w-full" pending={submitting} pendingLabel="Submitting…" disabled={readyProfiles.length === 0 || data.models.length === 0}><Play aria-hidden="true" /> Submit task</SubmitButton>
      </form>
    </Card.Content></Card.Root>
  </div>
</div>
