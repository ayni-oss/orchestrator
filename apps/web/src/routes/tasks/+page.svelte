<script lang="ts">
  import { page } from "$app/state";
  import ThemedSection from "$lib/components/ui/collapsible/section.svelte";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import { Checkbox } from "$lib/components/ui/checkbox/index.js";
  import { enhance } from "$lib/forms.js";
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

  let { data }: PageProps = $props();
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

  <div class="grid items-start gap-6 xl:grid-cols-[minmax(0,1fr)_24rem]">
    <section class="space-y-4"><div><h2 class="text-lg font-semibold">Task history</h2><p class="text-sm text-muted-foreground">Status and worker phase show delivery progress. Quality and publication have separate outcomes.</p></div>
      {#if data.tasks.length === 0}<EmptyState title="No tasks submitted" description="Submit bounded work against a ready runtime profile." />{:else}
        <div class="overflow-x-auto rounded-lg border bg-card"><Table.Root><Table.Header><Table.Row><Table.Head>Request</Table.Head><Table.Head>Status</Table.Head><Table.Head>Phase</Table.Head><Table.Head>Repository revision</Table.Head><Table.Head>Runtime digest</Table.Head><Table.Head>Updated</Table.Head></Table.Row></Table.Header><Table.Body>{#each data.tasks as task (task.id)}<Table.Row><Table.Cell><a class="font-medium hover:underline" href={`/tasks/${encodeURIComponent(task.id)}`}>{task.snapshot.request}</a><span class="block text-xs text-muted-foreground">{task.snapshot.agent.model}</span></Table.Cell><Table.Cell><StatusBadge status={task.state} tone={statusTone(task.state)} /></Table.Cell><Table.Cell>{words(data.phases[task.id] ?? "unavailable")}</Table.Cell><Table.Cell><CodeValue value={task.snapshot.repository.revision.slice(0, 12)} label={task.snapshot.repository.revision} /></Table.Cell><Table.Cell><CodeValue value={task.snapshot.runtime.resolvedDigest.slice(0, 19)} label={task.snapshot.runtime.resolvedDigest} /></Table.Cell><Table.Cell>{formatDate(task.updatedAt)}</Table.Cell></Table.Row>{/each}</Table.Body></Table.Root></div>
        {#if nextPageHref}<div class="flex justify-end"><Button href={nextPageHref} variant="outline">Next page</Button></div>{/if}
      {/if}
    </section>

    <Card.Root id="new" class="xl:sticky xl:top-24"><Card.Header><Card.Title>Submit task</Card.Title><Card.Description>Only operational, ready profiles can admit work.</Card.Description></Card.Header><Card.Content>
      {#if readyProfiles.length === 0 || data.models.length === 0}<Feedback kind="warning" title="Submission unavailable" message={readyProfiles.length === 0 ? "No runtime profile is ready." : "No provider model is available."} />{/if}
      <form data-refresh-paused={submitting ? "" : undefined} method="POST" action="?/create" class="mt-4 space-y-4" use:enhance={() => { submitting = true; return async ({ update }) => { try { await update(); } finally { submitting = false; } }; }}>
        <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="provider" value={data.provider ?? ""} /><input type="hidden" name="connectionId" value={data.connectionId ?? ""} />
        <div class="space-y-2"><Label for="project">Project</Label><ThemedSelect id="project" name="projectId" required value={selectedProject} onValueChange={value => { projectChoice = value; repositoryChoice = ""; selectedProfile = ""; }} placeholder="Select a project" options={data.projects.map(item => ({ value: item.id, label: item.displayName }))} /></div>
        <div class="space-y-2"><Label for="repository">Repository</Label><ThemedSelect id="repository" name="repositoryConnectionId" required value={selectedRepository} onValueChange={value => { repositoryChoice = value; selectedProfile = ""; }} placeholder="Select a repository" options={members.map(id => ({ value: id, label: id }))} /></div>
        <div class="space-y-2"><Label for="profile">Runtime profile</Label><ThemedSelect id="profile" name="runtimeProfileId" required bind:value={selectedProfile} disabled={readyProfiles.length === 0} placeholder="Select a ready profile" options={readyProfiles.map(profile => ({ value: profile.id, label: words(profile.kind) }))} /></div>
        <div class="space-y-2"><Label for="model">Model</Label><ThemedSelect id="model" name="model" required placeholder="Select a model" options={data.models.map(model => ({ value: model.id, label: model.name ?? model.id }))} /></div>
        <p class="text-xs text-muted-foreground">The selected profile’s verified repository revision and image digest are frozen at submission. Later builds do not change this request.</p>
        {#each repositoryProfiles as profile (profile.id)}{#if profile.state!=="ready"}<p class="text-sm"><a class="underline" href={`/profiles/${encodeURIComponent(profile.id)}`}>{words(profile.kind)}: {words(profile.state)} — view next step</a></p>{/if}{/each}
        <p class="text-sm text-muted-foreground">The worker codes, commits and checks the full repository, then makes bounded quality repairs. Valid changed work is published as a draft with truthful check results.</p>
        <div class="space-y-2"><Label for="request">Request</Label><Textarea id="request" name="request" required maxlength={16384} rows={4} /></div>
        <div class="space-y-2"><Label for="criteria">Acceptance criteria</Label><Textarea id="criteria" name="acceptanceCriteria" required rows={3} placeholder="One criterion per line" /></div>
        <ThemedSection title="Constraints and deadline"><div class="mt-3 space-y-3"><Textarea name="protectedPaths" rows={2} placeholder="Protected paths, one per line" /><label class="flex gap-2 text-sm"><Checkbox name="allowTestEdits" /> Allow test edits</label><Input name="deadlineSeconds" type="number" min={1} max={86400} placeholder="Deadline seconds" /></div></ThemedSection>
        <SubmitButton class="w-full" pending={submitting} pendingLabel="Submitting…" disabled={readyProfiles.length === 0 || data.models.length === 0}><Play aria-hidden="true" /> Submit task</SubmitButton>
      </form>
    </Card.Content></Card.Root>
  </div>
</div>
