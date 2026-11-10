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
  import SaveStatus from '$lib/components/save-status.svelte';
  import { protectUnsavedChanges } from '$lib/unsaved-changes.svelte';
  import { rpc, RpcError, type Connection } from '$lib/api';

  let { workspaceId, projectId }: { workspaceId: string; projectId: string } = $props();
  const budgets = [
    { key: 'maxInputTokens', id: 'max-input', label: 'Input token budget', min: 1, max: 1000000 },
    { key: 'maxOutputTokens', id: 'max-output', label: 'Output token budget', min: 1, max: 100000 },
    { key: 'maxTotalTokens', id: 'max-total', label: 'Total token budget', min: 2, max: 1100000 },
    { key: 'idleTimeoutSeconds', id: 'idle-timeout', label: 'Idle timeout (seconds)', min: 30, max: 86400 },
  ] as const;
  type Draft = Record<typeof budgets[number]['key'] | 'connectionId' | 'model' | 'instructions', string>;
  type Configuration = Draft & { workspaceId: string; revisionId: string; provider: string };
  type Update = { projectId: string; idempotencyKey: string; expectedRevisionId: string; configuration: Configuration };

  const defaultDraft: Draft = {
    connectionId: '', model: 'scripted', instructions: '',
    maxInputTokens: '8192', maxOutputTokens: '2048', maxTotalTokens: '10240', idleTimeoutSeconds: '300',
  };
  function normalized(value: Configuration): Configuration {
    // Protobuf JSON omits default-valued strings. Editor and recovery drafts are complete.
    return { ...defaultDraft, ...value, connectionId: value.connectionId ?? '', instructions: value.instructions ?? '' };
  }
  let configuration = $state.raw<Configuration | null>(null);
  let draft = $state<Draft>({ ...defaultDraft });
  let connections = $state.raw<Connection[]>([]);
  let loading = $state(true);
  let saving = $state(false);
  let error = $state('');
  let pending = $state.raw<Update | null>(null);
  let confirmed = $state<Partial<Record<keyof Draft, number>>>({});
  let timer: ReturnType<typeof setTimeout> | undefined;
  let due = false;
  let disposed = false;

  const fields = Object.keys(draft) as (keyof Draft)[];
  const storageKey = $derived(`ayni.project.${projectId}.configuration.pending`);
  const dirty = $derived(!!configuration && fields.some(key => draft[key] !== configuration?.[key]));
  protectUnsavedChanges(() => saving || (pending ? fields.some(key => draft[key] !== pending?.configuration[key]) : dirty));
  const selected = $derived(connections.find(connection => connection.id === draft.connectionId));
  const models = $derived(selected?.models ?? []);
  const validationMessage = $derived.by(() => {
    for (const { key, label, min, max } of budgets) {
      const value = Number(draft[key]);
      if (!Number.isInteger(value) || value < min || value > max) return `${label} must be a whole number between ${min} and ${max}.`;
    }
    if (Number(draft.maxTotalTokens) < Number(draft.maxInputTokens) + Number(draft.maxOutputTokens)) return 'Total token budget must cover the input and output budgets.';
    if (new TextEncoder().encode(draft.instructions).length > 16384) return 'Instructions are too long. Shorten the text before saving.';
    if (draft.connectionId) {
      if (!selected?.enabled || !selected.activeCredentialRevision) return 'This connection is unavailable. Choose an enabled, validated connection to save changes.';
      if (!models.includes(draft.model)) return 'Select a discovered model to save the connection.';
    } else if (configuration && configuration.provider !== 'scripted') {
      return 'Select a connection and discovered model to edit these provider settings.';
    }
    return '';
  });

  function applyDraft(value: Configuration) {
    for (const key of fields) draft[key] = value[key];
  }

  async function load() {
    loading = true;
    error = '';
    try {
      const result = await rpc('GetAgentConfiguration', { workspaceId, projectId, revisionId: '' });
      configuration = normalized(result.configuration);
      applyDraft(pending?.configuration ?? configuration);
      const values: Connection[] = [];
      let afterId = '';
      const visited = new Set<string>();
      do {
        const result = await rpc('ListConnections', { afterId, limit: 100 });
        values.push(...(result.connections ?? []).filter((connection: Connection) => connection.capability === 'llm'));
        afterId = result.nextAfterId || '';
        if (afterId && visited.has(afterId)) throw new Error('Connection pagination did not advance.');
        visited.add(afterId);
      } while (afterId);
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
    if (validationMessage) return;
    due = false;
    if (!fields.some(key => draft[key] !== configuration?.[key])) return;
    pending = {
      projectId,
      idempotencyKey: crypto.randomUUID(),
      expectedRevisionId: configuration.revisionId,
      configuration: { ...draft, workspaceId, revisionId: '', provider: draft.connectionId ? 'openai' : configuration.provider },
    };
    try {
      sessionStorage.setItem(storageKey, JSON.stringify(pending));
    } catch {
      pending = null;
      error = 'Browser storage is unavailable. Keep this page open and enable storage before saving.';
      return;
    }
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
      const saved = normalized(result.configuration);
      configuration = saved;
      for (const key of fields) {
        if (previous?.[key] !== saved[key] && draft[key] === request.configuration[key]) {
          confirmed[key] = Date.now();
        }
      }
      clearPending();
    } catch (cause) {
      if (cause instanceof RpcError && [400, 404, 409].includes(cause.status)) clearPending();
      due = false;
      error = cause instanceof RpcError && cause.status === 409 ? 'Settings changed elsewhere. Reload the saved settings before applying your changes again.' : cause instanceof Error ? cause.message : 'Configuration could not be saved.';
    } finally {
      saving = false;
      if (!pending) void save();
    }
  }

  onMount(() => {
    const stored = sessionStorage.getItem(storageKey);
    if (stored) {
      try {
        const request = JSON.parse(stored);
        if (request.projectId !== projectId || request.configuration?.workspaceId !== workspaceId || typeof request.idempotencyKey !== 'string' || typeof request.expectedRevisionId !== 'string' || !fields.every(key => typeof request.configuration[key] === 'string')) throw new Error('Invalid saved update');
        pending = request;
      } catch { sessionStorage.removeItem(storageKey); }
    }
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
        <Button variant="outline" onclick={() => { clearPending(); due = false; void load(); }}>Close request and reload</Button>
      </div>
    </InformationCard>
  {/if}
  {#if error && !pending}<Button variant="outline" onclick={() => { due = false; void load(); }}>Reload saved settings</Button>{/if}
  {#if loading}<p role="status">Loading project settings…</p>{/if}
  {#if configuration}
    <InformationCard title="Agent configuration">
      <p class="text-sm text-muted-foreground">Instructions, model and budgets apply to this project. Connections are shared; settings and conversation history stay separate.</p>
      <SaveStatus {saving} {dirty} pending={!!pending} saved={Object.keys(confirmed).length > 0} />
      <fieldset class="space-y-5" disabled={loading || (!!pending && !saving)}>
        <div class="space-y-2">
          {#snippet connectionControl()}
            <NativeSelect.Root id="project-connection" class="w-full" value={draft.connectionId} onchange={event => edit('connectionId', event.currentTarget.value)}>
              <NativeSelect.Option value="" disabled>Select a connection</NativeSelect.Option>
              {#each connections as connection (connection.id)}
                <NativeSelect.Option value={connection.id} disabled={!connection.enabled || !connection.activeCredentialRevision}>
                  {connection.displayName}{!connection.enabled ? ' (disabled)' : !connection.activeCredentialRevision ? ' (validation required)' : ''}
                </NativeSelect.Option>
              {/each}
            </NativeSelect.Root>
          {/snippet}
          {@render field('project-connection', 'OpenAI connection', 'connectionId', connectionControl)}
          <p class="text-xs text-muted-foreground"><a class="underline" href="/settings/llm-providers/openai">Manage OpenAI connections</a></p>
        </div>
        {#snippet modelControl()}
          <NativeSelect.Root id="project-model" class="w-full" value={draft.model === 'scripted' ? '' : draft.model} onchange={event => edit('model', event.currentTarget.value)} disabled={!selected}>
            <NativeSelect.Option value="" disabled>Select a discovered model</NativeSelect.Option>
            {#each models as model (model)}<NativeSelect.Option value={model}>{model}</NativeSelect.Option>{/each}
          </NativeSelect.Root>
        {/snippet}
        {@render field('project-model', 'Model', 'model', modelControl)}
        {#snippet instructionsControl()}
          <Textarea id="project-instructions" value={draft.instructions} oninput={event => edit('instructions', event.currentTarget.value)} maxlength={16384} rows={5} />
        {/snippet}
        {@render field('project-instructions', 'Instructions', 'instructions', instructionsControl)}
        <div class="grid gap-4 sm:grid-cols-2">
          {#each budgets as { key, id, label, min, max } (key)}
            {#snippet numberControl()}
              <Input {id} type="number" {min} {max} value={draft[key]} oninput={event => edit(key, event.currentTarget.value)} required />
            {/snippet}
            {@render field(id, label, key, numberControl)}
          {/each}
        </div>
        {#if !loading && validationMessage}<p role="alert" class="text-sm text-destructive">{validationMessage}</p>{/if}
      </fieldset>
    </InformationCard>
  {/if}
</div>
