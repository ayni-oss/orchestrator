<script lang="ts">
  import { tick } from "svelte";
  import ThemedSection from "$lib/components/ui/collapsible/section.svelte";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import { Checkbox } from "$lib/components/ui/checkbox/index.js";
  import { enhance } from "$lib/forms.js";
  import type { PageData } from "../../routes/projects/[projectId]/$types";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { Textarea } from "$lib/components/ui/textarea/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { words } from "$lib/presentation.js";
  let {project,repositories,profiles,models,provider,idempotencyKey,onSubmitted} : {project:NonNullable<PageData["project"]>;repositories:PageData["repositories"];profiles:PageData["profiles"];models:PageData["models"];provider:string;idempotencyKey:string;onSubmitted?:()=>Promise<void>} = $props();
  let choice=$state("");
  const repository=$derived(project.repositoryConnectionIds.includes(choice) ? choice : project.repositoryConnectionIds.length===1 ? project.repositoryConnectionIds[0]! : "");
  const ready=$derived(profiles.filter(profile => profile.repositoryConnectionId===repository && profile.support==="operational" && profile.state==="ready"));
  let selectedProfile=$state("");
  $effect(() => {
    if (!ready.some(profile => profile.id === selectedProfile)) selectedProfile = ready.length === 1 ? ready[0]!.id : "";
  });
  let pending=$state(false), allowTestEdits=$state(true);
</script>
<form action="?/createTask" method="POST" class="space-y-4" use:enhance={() => {pending=true;return async ({update,result})=>{pending=false;if(result.type === "redirect") {await onSubmitted?.();await tick();}await update();};}}>
  <input type="hidden" name="idempotencyKey" value={idempotencyKey} /><input type="hidden" name="projectId" value={project.id} /><input type="hidden" name="provider" value={provider} />
  <div class="space-y-2">
    <Label for="task-repository">Repository</Label>
    <ThemedSelect id="task-repository" descriptionId="task-repository-help" name="repositoryConnectionId" required value={repository} onValueChange={value => { choice = value; selectedProfile = ""; }} placeholder="Select a repository" options={repositories.map(repo => ({ value: repo.id, label: repo.displayName }))} />
    <p id="task-repository-help" class="text-xs text-muted-foreground">The repository the agent will change. Choose one repository from this project.</p>
  </div>
  <div class="space-y-2">
    <Label for="task-profile">Runtime profile</Label>
    <ThemedSelect id="task-profile" descriptionId="task-profile-help" name="runtimeProfileId" required bind:value={selectedProfile} placeholder="Select a ready runtime" options={ready.map(profile => ({ value: profile.id, label: words(profile.kind) }))} />
    <p id="task-profile-help" class="text-xs text-muted-foreground">The prepared environment with tools and dependencies for coding and checks. Its source revision and image are fixed when you submit.</p>
  </div>
  {#if repository && !ready.length}<p role="status" class="text-sm text-muted-foreground">This repository needs a ready runtime before you can submit work.</p>{/if}
  <div class="space-y-2">
    <Label for="task-model">Model</Label>
    <ThemedSelect id="task-model" descriptionId="task-model-help" name="model" required placeholder="Select a model" options={models.map(model => ({ value: model.id, label: model.id }))} />
    <p id="task-model-help" class="text-xs text-muted-foreground">The AI model that implements the request and repairs check failures. Choose gpt-5.6-luna, gpt-5.6-terra, or gpt-5.6-sol when available from your provider.</p>
  </div>
  {#if !models.length}<p class="text-sm text-muted-foreground">No supported models are available. Check provider access to gpt-5.6-luna, gpt-5.6-terra, and gpt-5.6-sol.</p>{/if}
  <div class="space-y-2">
    <Label for="task-request">Request</Label><Textarea id="task-request" aria-describedby="task-request-help" name="request" required maxlength={16384} rows={4} />
    <p id="task-request-help" class="text-xs text-muted-foreground">Describe the problem, the change you want, and any context the agent needs.</p>
  </div>
  <div class="space-y-2">
    <Label for="task-criteria">Acceptance criteria</Label><Textarea id="task-criteria" aria-describedby="task-criteria-help" name="acceptanceCriteria" required rows={3} placeholder="One criterion per line" />
    <p id="task-criteria-help" class="text-xs text-muted-foreground">List observable outcomes, one per line. These guide the agent; mandatory Ayni checks run separately and do not automatically verify every criterion.</p>
  </div>
  <div class="space-y-2">
    <Label for="task-iterations">Agent iterations</Label><Input id="task-iterations" aria-describedby="task-iterations-help" name="codingTurns" type="number" min={1} max={128} value={24} required />
    <p id="task-iterations-help" class="text-xs text-muted-foreground">Maximum model turns for this task, including implementation and quality repairs. Each turn lets the model respond or request tools. Default: 24; choose 1–128. Work can finish earlier.</p>
  </div>
  <ThemedSection title="Constraints and deadline">
    <div class="mt-3 space-y-4">
      <div class="space-y-2">
        <Label for="task-protected">Protected paths</Label><Textarea id="task-protected" aria-describedby="task-protected-help" name="protectedPaths" rows={2} placeholder="One path per line" />
        <p id="task-protected-help" class="text-xs text-muted-foreground">Files or directories the delivered changes must leave untouched, relative to the repository root. One path per line; no wildcards. Ayni policy and lock files are always protected.</p>
      </div>
      <div class="space-y-2">
        <label class="flex gap-2 text-sm"><Checkbox name="allowTestEdits" bind:checked={allowTestEdits} aria-describedby="task-tests-help" /> Allow test edits</label>
        <p id="task-tests-help" class="text-xs text-muted-foreground">Enabled by default so the agent can add and update tests with its implementation. Turn off to protect recognized test files. Ayni checks still run either way.</p>
      </div>
      <div class="space-y-2">
        <Label for="task-deadline">Deadline (seconds)</Label><Input id="task-deadline" aria-describedby="task-deadline-help" name="deadlineSeconds" type="number" min={1} max={86400} placeholder="3600 (1 hour)" />
        <p id="task-deadline-help" class="text-xs text-muted-foreground">Total time from submission, including queueing, coding, checks, and delivery. Leave blank for 1 hour; maximum 24 hours. Some time is reserved for checks and publication.</p>
      </div>
    </div>
  </ThemedSection>
  <p class="text-xs text-muted-foreground">Work targets the selected repository and is published as a draft PR with its actual check results.</p>
  <Button type="submit" class="w-full" disabled={pending || !ready.length || !models.length}>{pending ? "Submitting…" : "Submit task"}</Button>
</form>
