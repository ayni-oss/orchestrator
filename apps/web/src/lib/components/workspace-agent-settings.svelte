<script lang="ts">
  import { onMount, onDestroy, type Snippet } from 'svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Textarea } from '$lib/components/ui/textarea/index.js';
  import * as NativeSelect from '$lib/components/ui/native-select/index.js';
  import { Label } from '$lib/components/ui/label/index.js';
  import InformationCard from '$lib/components/information-card.svelte';
  import Feedback from '$lib/components/feedback.svelte';
  import FieldSaved from '$lib/components/field-saved.svelte';
  import { rpc, type Connection } from '$lib/api';

  let { workspaceId }: { workspaceId: string } = $props();
  const budgets = [
    { key: 'maxInputTokens', id: 'max-input', label: 'Input token budget', min: 1, max: 1000000 },
    { key: 'maxOutputTokens', id: 'max-output', label: 'Output token budget', min: 1, max: 100000 },
    { key: 'maxTotalTokens', id: 'max-total', label: 'Total token budget', min: 2, max: 1100000 },
    { key: 'idleTimeoutSeconds', id: 'idle-timeout', label: 'Idle timeout (seconds)', min: 30, max: 86400 },
  ] as const;
  type Draft = Record<typeof budgets[number]['key'] | 'connectionId' | 'model' | 'instructions', string>;
  type Configuration = Draft & { workspaceId: string; revisionId: string; provider: string };
  type Update = { idempotencyKey: string; expectedRevisionId: string; configuration: Configuration };

  let configuration = $state.raw<Configuration | null>(null);
  let draft = $state<Draft>({
    connectionId: '', model: '', instructions: '',
    maxInputTokens: '8192', maxOutputTokens: '2048', maxTotalTokens: '10240', idleTimeoutSeconds: '300',
  });
  let connections = $state.raw<Connection[]>([]);
  let loading = $state(true);
  let saving = $state(false);
  let error = $state('');
  let pending = $state.raw<Update | null>(null);
  let confirmed = $state<Partial<Record<keyof Draft, number>>>({});
  let timer: ReturnType<typeof setTimeout> | undefined;
  let due = false;
  let disposed = false;

  const storageKey = $derived(`ayni.workspace.${workspaceId}.configuration.pending`);
  const selected = $derived(connections.find(connection => connection.id === draft.connectionId));
  const models = $derived(selected?.models ?? []);
  const overBudget = $derived(Number(draft.maxTotalTokens) < Number(draft.maxInputTokens) + Number(draft.maxOutputTokens));
  const fields = Object.keys(draft) as (keyof Draft)[];

  function applyDraft(value: Configuration) {
    for (const key of fields) draft[key] = value[key];
    if (draft.model === 'scripted') draft.model = '';
  }

  async function load() {
    loading = true;
    error = '';
    try {
      const result = await rpc('GetAgentConfiguration', { workspaceId, revisionId: '' });
      configuration = result.configuration;
      applyDraft(pending?.configuration ?? result.configuration);
      const values: Connection[] = [];
      let afterId = '';
      let pages = 0;
      do {
        const result = await rpc('ListConnections', { afterId, limit: 100 });
        values.push(...(result.connections ?? []).filter((connection: Connection) => connection.capability === 'llm'));
        afterId = result.nextAfterId || '';
        pages++;
      } while (afterId && pages < 20);
      connections = values;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : 'Configuration unavailable.';
    } finally {
      loading = false;
    }
  }

  function edit(key: keyof Draft, value: string) {
    draft[key] = value;
    delete confirmed[key];
    if (key === 'connectionId' && !models.includes(draft.model)) {
      draft.model = '';
      delete confirmed.model;
    }
    clearTimeout(timer);
    due = false;
    timer = setTimeout(() => {
      due = true;
      void save();
    }, 500);
  }

  async function save() {
    if (disposed || !due || saving || pending || !configuration) return;
    if (!selected?.enabled || !selected.activeCredentialRevision || !models.includes(draft.model) || overBudget) return;
    if (budgets.some(({ key, min, max }) => {
      const value = Number(draft[key]);
      return !Number.isInteger(value) || value < min || value > max;
    })) return;
    due = false;
    if (!fields.some(key => draft[key] !== configuration?.[key])) return;
    pending = {
      idempotencyKey: crypto.randomUUID(),
      expectedRevisionId: configuration.revisionId,
      configuration: { ...draft, workspaceId, revisionId: '', provider: 'openai' },
    };
    sessionStorage.setItem(storageKey, JSON.stringify(pending));
    await recover();
  }

  function clearPending() {
    pending = null;
    sessionStorage.removeItem(storageKey);
  }

  async function recover() {
    const request = pending;
    if (!request || saving) return;
    saving = true;
    error = '';
    try {
      const result = await rpc('UpdateAgentConfiguration', request);
      const previous = configuration;
      const saved: Configuration = result.configuration;
      configuration = saved;
      for (const key of fields) {
        if (previous?.[key] !== saved[key] && draft[key] === request.configuration[key]) {
          confirmed[key] = Date.now();
        }
      }
      clearPending();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : 'Configuration could not be saved.';
    } finally {
      saving = false;
      if (!pending) void save();
    }
  }

  onMount(() => {
    const stored = sessionStorage.getItem(storageKey);
    if (stored) pending = JSON.parse(stored);
    void load();
  });
  onDestroy(() => {
    disposed = true;
    clearTimeout(timer);
  });
</script>

{#snippet field(id: string, label: string, key: keyof Draft, control: Snippet)}
  <div class="space-y-2">
    <Label for={id}>{label}</Label>
    <div class="relative">
      {@render control()}
      <FieldSaved revision={confirmed[key]} />
    </div>
  </div>
{/snippet}

<div class="space-y-6">
  {#if error}<Feedback kind="error" title="Configuration needs attention" message={error} />{/if}
  {#if pending && !saving && !loading}
    <InformationCard title="Recover a configuration update">
      <p>Retry the retained command to recover its original result.</p>
      <div class="flex flex-wrap gap-3">
        <Button onclick={recover}>Recover update</Button>
        <Button variant="outline" onclick={() => { clearPending(); due = false; void load(); }}>Discard unsaved changes</Button>
      </div>
    </InformationCard>
  {/if}
  {#if configuration}
    <InformationCard title="Agent configuration">
      <fieldset class="space-y-5" disabled={loading || (!!pending && !saving)}>
        <div class="space-y-2">
          {#snippet connectionControl()}
            <NativeSelect.Root id="workspace-connection" class="w-full" value={draft.connectionId} onchange={event => edit('connectionId', event.currentTarget.value)}>
              <NativeSelect.Option value="" disabled>Select a connection</NativeSelect.Option>
              {#each connections as connection (connection.id)}
                <NativeSelect.Option value={connection.id} disabled={!connection.enabled || !connection.activeCredentialRevision}>
                  {connection.displayName}{!connection.enabled ? ' (disabled)' : !connection.activeCredentialRevision ? ' (validation required)' : ''}
                </NativeSelect.Option>
              {/each}
            </NativeSelect.Root>
          {/snippet}
          {@render field('workspace-connection', 'OpenAI connection', 'connectionId', connectionControl)}
          <p class="text-xs text-muted-foreground"><a class="underline" href="/settings/llm-providers/openai">Manage OpenAI connections</a></p>
        </div>
        {#snippet modelControl()}
          <NativeSelect.Root id="workspace-model" class="w-full" value={draft.model} onchange={event => edit('model', event.currentTarget.value)} disabled={!selected}>
            <NativeSelect.Option value="" disabled>Select a discovered model</NativeSelect.Option>
            {#each models as model (model)}<NativeSelect.Option value={model}>{model}</NativeSelect.Option>{/each}
          </NativeSelect.Root>
        {/snippet}
        {@render field('workspace-model', 'Model', 'model', modelControl)}
        {#if draft.connectionId !== configuration.connectionId && !draft.model}
          <p class="text-sm text-muted-foreground">Select a model to save the new connection.</p>
        {/if}
        {#snippet instructionsControl()}
          <Textarea id="workspace-instructions" value={draft.instructions} oninput={event => edit('instructions', event.currentTarget.value)} maxlength={16384} rows={5} />
        {/snippet}
        {@render field('workspace-instructions', 'Instructions', 'instructions', instructionsControl)}
        <div class="grid gap-4 sm:grid-cols-2">
          {#each budgets as { key, id, label, min, max } (key)}
            {#snippet numberControl()}
              <Input {id} type="number" {min} {max} value={draft[key]} oninput={event => edit(key, event.currentTarget.value)} required />
            {/snippet}
            {@render field(id, label, key, numberControl)}
          {/each}
        </div>
        {#if overBudget}<p class="text-sm text-destructive">Total token budget must cover the input and output budgets.</p>{/if}
      </fieldset>
    </InformationCard>
  {/if}
</div>
