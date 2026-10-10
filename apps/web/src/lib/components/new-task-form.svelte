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
  let pending=$state(false);
</script>
<form action="?/createTask" method="POST" class="space-y-4" use:enhance={() => {pending=true;return async ({update,result})=>{pending=false;if(result.type === "redirect") {await onSubmitted?.();await tick();}await update();};}}>
  <input type="hidden" name="idempotencyKey" value={idempotencyKey} /><input type="hidden" name="projectId" value={project.id} /><input type="hidden" name="provider" value={provider} />
  <div class="space-y-2"><Label for="task-repository">Repository</Label><ThemedSelect id="task-repository" name="repositoryConnectionId" required value={repository} onValueChange={value => { choice = value; selectedProfile = ""; }} placeholder="Select a repository" options={repositories.map(repo => ({ value: repo.id, label: repo.displayName }))} /></div>
  <div class="space-y-2"><Label for="task-profile">Runtime profile</Label><ThemedSelect id="task-profile" name="runtimeProfileId" required bind:value={selectedProfile} placeholder="Select a ready runtime" options={ready.map(profile => ({ value: profile.id, label: words(profile.kind) }))} /></div>
  {#if repository && !ready.length}<p role="status" class="text-sm text-muted-foreground">This repository needs a ready runtime before you can submit work.</p>{/if}
  <div class="space-y-2"><Label for="task-model">Model</Label><ThemedSelect id="task-model" name="model" required placeholder="Select a model" options={models.map(model => ({ value: model.id, label: model.name ?? model.id }))} /></div>
  {#if !models.length}<p class="text-sm text-muted-foreground">Models are unavailable. Check provider configuration and try again.</p>{/if}
  <div class="space-y-2"><Label for="task-request">Request</Label><Textarea id="task-request" name="request" required maxlength={16384} rows={4} /></div>
  <div class="space-y-2"><Label for="task-criteria">Acceptance criteria</Label><Textarea id="task-criteria" name="acceptanceCriteria" required rows={3} placeholder="One criterion per line" /></div>
  <ThemedSection title="Constraints and deadline"><div class="mt-3 space-y-3"><Label for="task-protected">Protected paths</Label><Textarea id="task-protected" name="protectedPaths" rows={2} placeholder="One path per line" /><label class="flex gap-2 text-sm"><Checkbox name="allowTestEdits" /> Allow test edits</label><Label for="task-deadline">Deadline (seconds)</Label><Input id="task-deadline" name="deadlineSeconds" type="number" min={1} max={86400} /></div></ThemedSection>
  <p class="text-xs text-muted-foreground">Work targets the selected repository and is published as a draft PR with its actual check results.</p>
  <Button type="submit" class="w-full" disabled={pending || !ready.length || !models.length}>{pending ? "Submitting…" : "Submit task"}</Button>
</form>
