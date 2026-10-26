<script lang="ts">
  import { untrack } from "svelte";
  import type { SubmitFunction } from "@sveltejs/kit";
  import type { ProfileRole } from "$lib/agent-profiles.js";
  import type { AgentCapabilitiesDto, AgentProfileSettingsDto } from "$lib/project-view.js";
  import { profileChanges } from "$lib/agent-profile-diff.js";
  import { enhance } from "$lib/forms.js";
  import InformationCard from "$lib/components/information-card.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Textarea } from "$lib/components/ui/textarea/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Checkbox from "$lib/components/ui/checkbox/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import * as Table from "$lib/components/ui/table/index.js";

  let { role, configured, inherited, effective, capabilities, repositories, project = false, idempotencyKey, resourceVersion }: {
    role: ProfileRole; configured?: AgentProfileSettingsDto | undefined; inherited?: AgentProfileSettingsDto | undefined;
    effective?: AgentProfileSettingsDto | undefined; capabilities: AgentCapabilitiesDto; repositories: string[];
    project?: boolean; idempotencyKey: string; resourceVersion: string;
  } = $props();
  const title = $derived(role === "planner" ? "Planner profile" : "Worker profile");
  const source = $derived(project ? "Workspace capabilities" : "Application capabilities");
  const defaults = $derived(inherited ?? capabilities.roles[role]);
  const limits = $derived(capabilities.limits);
  const clone = (value: AgentProfileSettingsDto): AgentProfileSettingsDto => JSON.parse(JSON.stringify(value));
  let baseline = $state(untrack(() => clone(configured ?? effective ?? defaults)));
  let draft = $state(untrack(() => clone(baseline)));
  let baselineInherited = $state(untrack(() => !configured));
  let useInherited = $state(untrack(() => baselineInherited));
  let pending = $state(false), reviewOpen = $state(false), error = $state(""), success = $state("");
  let form: HTMLFormElement;
  let confirmed = false, reviewVersion = "";
  let skillKeys = $state(untrack(() => draft.skills.map((_, index) => index)));
  let nextSkillKey = untrack(() => skillKeys.length);
  const changes = $derived(profileChanges(baseline, draft, baselineInherited, useInherited, source, project));
  function inherit(value: boolean) {
    useInherited = value;
    draft = clone(defaults);
    if (!value && project) draft.instructions = "";
    skillKeys = draft.skills.map(() => nextSkillKey++);
    error = ""; success = "";
  }
  function toggle(values: string[], value: string, checked: boolean) {
    return checked ? [...values.filter(item => item !== value), value] : values.filter(item => item !== value);
  }
  const submit: SubmitFunction = ({ cancel, formData }) => {
    if (!confirmed) {
      cancel();
      if (changes.length) { reviewVersion = resourceVersion; reviewOpen = true; error = ""; success = ""; }
      return;
    }
    confirmed = false;
    formData.set("resourceVersion", reviewVersion);
    pending = true;
    return async ({ result, update }) => {
      try {
        if (result.type === "failure") error = String(result.data?.error?.message ?? "Profile could not be saved. Try again.");
        if (result.type === "error") error = result.error.message || "Profile could not be saved. Try again.";
        await update({ reset: false });
        if (result.type === "success") {
          baseline = clone(draft); baselineInherited = useInherited;
          success = String(result.data?.success ?? `${title} saved.`);
          reviewOpen = false;
        }
      } finally { pending = false; }
    };
  };
  function confirm() { confirmed = true; form.requestSubmit(); }
</script>

<InformationCard {title}>
  <form bind:this={form} aria-label={title} method="POST" action="?/agentProfile" use:enhance={submit} class="space-y-5" data-refresh-paused={changes.length || reviewOpen || pending ? "" : undefined}>
    <input type="hidden" name="role" value={role} />
    <input type="hidden" name="idempotencyKey" value={idempotencyKey} />
    <input type="hidden" name="resourceVersion" value={resourceVersion} />
    <div class="flex items-center gap-2">
      <Checkbox.Root id={`${role}-inherit`} name={`${role}.inherit`} value="true" checked={useInherited} onCheckedChange={inherit} disabled={pending} />
      <Label for={`${role}-inherit`}>{project ? "Use workspace capabilities" : "Use application capabilities"}</Label>
    </div>
    <p class="text-sm text-muted-foreground">{role === "planner" ? "Research tools read selected repositories and prepare proposals. They cannot edit repositories or execute commands." : "Worker tools operate inside the approved task and sandbox. The final quality check is always required."}</p>
    <fieldset disabled={useInherited || pending} class="min-w-0 space-y-5 disabled:opacity-70">
      <legend class="sr-only">{title} settings</legend>
      <div class="space-y-2">
        <Label for={`${role}-instructions`}>{project && !useInherited ? "Additional instructions" : "Instructions"}</Label>
        <Textarea id={`${role}-instructions`} name={`${role}.instructions`} bind:value={draft.instructions} maxlength={limits.maxInstructionBytes} rows={3} placeholder="Application role instructions" />
        <p class="text-xs text-muted-foreground">Instructions and skills provide context. They cannot add permissions. Do not include credentials.</p>
      </div>
      <div class="grid gap-4 sm:grid-cols-2">
        <div class="space-y-2"><Label for={`${role}-calls`}>Maximum model calls</Label><Input id={`${role}-calls`} name={`${role}.calls`} type="number" min={1} max={project ? defaults.maxModelCalls : limits.maxModelCalls} bind:value={draft.maxModelCalls} required /></div>
        <div class="space-y-2"><Label for={`${role}-deadline`}>Maximum duration (seconds)</Label><Input id={`${role}-deadline`} name={`${role}.deadline`} type="text" inputmode="numeric" pattern="[0-9]+" bind:value={draft.deadlineSeconds} required /></div>
      </div>
      <fieldset class="space-y-3">
        <legend class="mb-2 text-sm font-medium">Allowed tools</legend>
        <div class="grid gap-3 sm:grid-cols-2">
          {#each defaults.allowedTools as tool (tool)}
            <div class="flex min-w-0 items-center gap-2"><Checkbox.Root id={`${role}-tool-${tool}`} name={`${role}.tools`} value={tool} checked={draft.allowedTools.includes(tool)} onCheckedChange={checked => draft.allowedTools = toggle(draft.allowedTools, tool, checked)} /><Label for={`${role}-tool-${tool}`} class="min-w-0 break-all font-mono text-xs">{tool}</Label></div>
          {/each}
        </div>
      </fieldset>
      <fieldset class="space-y-3">
        <legend class="mb-2 text-sm font-medium">Repository scope</legend>
        <p class="text-xs text-muted-foreground">Leave unselected to use current repository membership.</p>
        {#each repositories as id (id)}
          <div class="flex min-w-0 items-center gap-2"><Checkbox.Root id={`${role}-repo-${id}`} name={`${role}.repositories`} value={id} checked={draft.repositoryIds.includes(id)} onCheckedChange={checked => draft.repositoryIds = toggle(draft.repositoryIds, id, checked)} /><Label for={`${role}-repo-${id}`} class="min-w-0 break-all text-xs">{id}</Label></div>
        {/each}
      </fieldset>
      <div class="space-y-3">
        <h4 class="text-sm font-medium">Pinned skills</h4>
        <p class="text-xs text-muted-foreground">Each skill is pinned to a revision. Project skills must match workspace revisions.</p>
        {#each draft.skills as skill, index (skillKeys[index])}
          <fieldset class="space-y-3 rounded-lg border p-4">
            <legend class="px-2 text-sm">Skill {index + 1}</legend>
            <div class="space-y-2"><Label for={`${role}-skill-${skillKeys[index]}-id`}>Skill ID</Label><Input id={`${role}-skill-${skillKeys[index]}-id`} name={`${role}.skill.${index}.id`} bind:value={skill.id} maxlength={limits.maxIdentifierBytes} required /></div>
            <div class="space-y-2"><Label for={`${role}-skill-${skillKeys[index]}-revision`}>Revision</Label><Input id={`${role}-skill-${skillKeys[index]}-revision`} name={`${role}.skill.${index}.revision`} bind:value={skill.revision} maxlength={limits.maxIdentifierBytes} required /></div>
            <div class="space-y-2"><Label for={`${role}-skill-${skillKeys[index]}-text`}>Skill instructions</Label><Textarea id={`${role}-skill-${skillKeys[index]}-text`} name={`${role}.skill.${index}.instructions`} bind:value={skill.instructions} maxlength={limits.maxInstructionBytes} rows={2} /></div>
            <Button type="button" variant="outline" onclick={() => { draft.skills.splice(index, 1); skillKeys.splice(index, 1); }}>Remove skill {index + 1}</Button>
          </fieldset>
        {:else}<p class="text-sm text-muted-foreground">No pinned skills.</p>{/each}
        {#if draft.skills.length < limits.maxSkills}<Button type="button" variant="outline" onclick={() => { draft.skills.push({ id: "", revision: "", instructions: "" }); skillKeys.push(nextSkillKey++); }}>Add pinned skill</Button>{/if}
      </div>
    </fieldset>
    <Button type="submit" disabled={pending || !changes.length}>{pending ? "Saving…" : `Save ${role} profile`}</Button>
    {#if error && !reviewOpen}<Feedback kind="error" title="Profile not saved" message={error} />{:else if success}<p role="status" class="text-sm">{success}</p>{/if}
  </form>
</InformationCard>
<Dialog.Root bind:open={reviewOpen}>
  <Dialog.Content class="max-h-[85vh] overflow-y-auto sm:max-w-3xl" showCloseButton={!pending} onEscapeKeydown={event => { if (pending) event.preventDefault(); }} onInteractOutside={event => { if (pending) event.preventDefault(); }}>
    <Dialog.Header><Dialog.Title>Review {role} profile changes</Dialog.Title><Dialog.Description>Compare the current settings with your changes. Saving applies to new executions.</Dialog.Description></Dialog.Header>
    <Table.Root class="table-fixed">
      <Table.Header><Table.Row><Table.Head class="w-1/4">Setting</Table.Head><Table.Head>Current</Table.Head><Table.Head>Proposed</Table.Head></Table.Row></Table.Header>
      <Table.Body>{#each changes as change (change.label)}<Table.Row><Table.Cell class="whitespace-normal align-top font-medium">{change.label}</Table.Cell><Table.Cell class="whitespace-pre-wrap break-words align-top text-muted-foreground [overflow-wrap:anywhere]">{change.before}</Table.Cell><Table.Cell class="whitespace-pre-wrap break-words bg-primary/5 align-top [overflow-wrap:anywhere]">{change.after}</Table.Cell></Table.Row>{/each}</Table.Body>
    </Table.Root>
    {#if error}<Feedback kind="error" title="Profile not saved" message={error} />{/if}
    <Dialog.Footer><Button type="button" variant="outline" disabled={pending} onclick={() => reviewOpen = false}>Back to editing</Button><Button type="button" disabled={pending} onclick={confirm}>{pending ? "Saving…" : "Confirm changes"}</Button></Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
