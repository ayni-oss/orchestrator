<script lang="ts">
  import { invalidateAll } from '$app/navigation';
  import { onDestroy, onMount } from 'svelte';
  import { toast } from 'svelte-sonner';
  import { rpc, RpcError } from '$lib/api';
  import AutosaveField from '$lib/components/autosave-field.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Label } from '$lib/components/ui/label/index.js';
  import { Textarea } from '$lib/components/ui/textarea/index.js';
  import { protectUnsavedChanges } from '$lib/unsaved-changes.svelte';
  import type { Project } from '$lib/workspace-types';

  let { project }: { project: Project } = $props();
  type GoalUpdate = {
    workspaceId: string;
    projectId: string;
    idempotencyKey: string;
    expectedResourceVersion: string;
    goal: string;
  };

  let draft = $state<string | null>(null);
  let editVersion = $state('');
  let pending = $state.raw<GoalUpdate | null>(null);
  let saving = $state(false);
  let savedRevision = $state(0);
  let error = $state('');
  let timer: ReturnType<typeof setTimeout> | undefined;
  let due = false;
  let disposed = false;

  const storageKey = $derived(`ayni.project.${project.id}.goal.pending`);
  const goal = $derived(project.goal ?? '');
  const dirty = $derived(draft !== null && draft !== goal);
  const tooLong = $derived(draft !== null && new TextEncoder().encode(draft).length > 16_384);
  protectUnsavedChanges(() => saving || (pending ? draft !== pending.goal : dirty));

  function stopTimer() {
    clearTimeout(timer);
    due = false;
  }

  function edit(value: string) {
    if (draft === null) editVersion = project.resourceVersion;
    draft = !pending && value === goal ? null : value;
    savedRevision = 0;
    error = '';
    stopTimer();
    timer = setTimeout(() => {
      due = true;
      void save();
    }, 500);
  }

  function clearPending() {
    sessionStorage.removeItem(storageKey);
    pending = null;
  }

  async function save() {
    if (disposed || !due || saving || pending || tooLong || !dirty || draft === null) return;
    due = false;
    const request = {
      workspaceId: project.workspaceId,
      projectId: project.id,
      idempotencyKey: crypto.randomUUID(),
      expectedResourceVersion: editVersion,
      goal: draft,
    };
    try {
      sessionStorage.setItem(storageKey, JSON.stringify(request));
      pending = request;
    } catch {
      error = 'Browser storage is unavailable. The goal could not be saved.';
      toast.error('Could not save the project goal', { description: error });
      return;
    }
    await submitPending();
  }

  async function submitPending() {
    const request = pending;
    if (!request || saving) return;
    saving = true;
    error = '';
    let acknowledged = false;
    try {
      await rpc('SetProjectGoal', request);
      acknowledged = true;
      await invalidateAll();
      editVersion = project.resourceVersion;
      if (goal !== request.goal) {
        stopTimer();
        error = 'The goal changed elsewhere. Review the latest goal before editing again.';
        if (draft === request.goal) draft = null;
      } else if (draft === request.goal) {
        draft = null;
        savedRevision = Date.now();
      }
    } catch (cause) {
      stopTimer();
      if (acknowledged) {
        error = 'The goal was saved. Reload this page to see the latest project settings.';
      } else if (cause instanceof RpcError && [400, 404, 409].includes(cause.status)) {
        clearPending();
        error = cause.message;
        if (cause.status === 409) {
          try {
            await invalidateAll();
            editVersion = project.resourceVersion;
            error = 'Project settings changed elsewhere. Review your goal, then edit it again to save.';
          } catch {
            error = 'Project settings changed elsewhere. Reload the page before editing again.';
          }
        }
      } else {
        error = 'The save result is unknown. Retry the same goal update.';
      }
    } finally {
      if (error) toast.error('Project goal needs attention', { description: error });
      if (acknowledged) clearPending();
      saving = false;
      if (!pending) void save();
    }
  }

  onMount(() => {
    const stored = sessionStorage.getItem(storageKey);
    if (!stored) return;
    try {
      const request: GoalUpdate = JSON.parse(stored);
      if (request.workspaceId !== project.workspaceId || request.projectId !== project.id ||
          typeof request.goal !== 'string' || typeof request.idempotencyKey !== 'string' ||
          typeof request.expectedResourceVersion !== 'string') throw new Error('Invalid goal update');
      draft = request.goal;
      editVersion = request.expectedResourceVersion;
      pending = request;
    } catch {
      clearPending();
    }
  });

  onDestroy(() => {
    disposed = true;
    stopTimer();
  });
</script>

<div class="space-y-2">
  <Label for="project-goal">Project Goal</Label>
  <AutosaveField saving={saving || (dirty && !pending && !tooLong && !error)} {savedRevision}>
    <Textarea data-slot="input-group-control" id="project-goal" rows={4} value={draft ?? goal} readonly={!!pending && !saving}
      oninput={event => edit(event.currentTarget.value)} aria-describedby="project-goal-help" />
  </AutosaveField>
  <p id="project-goal-help" class="text-sm text-muted-foreground">Describe the bigger picture. New tasks receive this goal alongside their own objective.</p>
  {#if tooLong}<p role="alert" class="text-sm text-destructive">The goal is too long. Shorten it before saving.</p>{/if}
  {#if pending && !saving}<Button onclick={submitPending}>Retry goal update</Button>{/if}
</div>
