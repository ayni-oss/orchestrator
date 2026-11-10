<script lang="ts">
  import { goto } from '$app/navigation';
  import { Plus, RefreshCw } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { toast } from 'svelte-sonner';
  import { rpc, RpcError, reasonText, type Connection, type Operation } from '$lib/api';
  import Feedback from '$lib/components/feedback.svelte';
  import InformationCard from '$lib/components/information-card.svelte';
  import InformationTable from '$lib/components/information-table.svelte';
  import PageContent from '$lib/components/page-content.svelte';
  import PageHeader from '$lib/components/page-header.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Label } from '$lib/components/ui/label/index.js';
  import { Textarea } from '$lib/components/ui/textarea/index.js';
  import { protectUnsavedChanges } from '$lib/unsaved-changes.svelte';

  type Capability = 'source_control' | 'container_registry' | 'llm';
  const surfaces = {
    source_control: {
      title: 'GitHub', section: 'Source control', label: 'GitHub App', path: '/settings/source-control/github',
      add: 'Add GitHub App', description: 'Add and manage the GitHub Apps that connect your repositories.',
      validation: 'Validation checks the app and its private key. Repository access depends on where the app is installed.',
    },
    container_registry: {
      title: 'GitHub', section: 'Container registries', label: 'GitHub Container Registry', path: '/settings/container-registries/github',
      add: 'Add GitHub Container Registry connection', description: 'Connect GitHub Container Registry to publish and pull runtime images.',
      validation: 'Validation checks the identity of the push and pull accounts. Runtime preparation checks package permissions.',
    },
    llm: {
      title: 'OpenAI', section: 'LLM providers', label: 'OpenAI', path: '/settings/llm-providers/openai',
      add: 'Add OpenAI connection', description: 'Connect OpenAI to make its models available in project settings.',
      validation: 'Validation checks the API key and discovers available models.',
    },
  };
  let { capability, connectionId, createOnMount = false, onSaved }: {
    capability: Capability; connectionId?: string; createOnMount?: boolean; onSaved?: (connection: Connection) => void;
  } = $props();
  const surface = $derived(surfaces[capability]);
  const github = $derived(capability === 'source_control');
  const registry = $derived(capability === 'container_registry');
  const indexPath = $derived(surface.path);
  const storageKey = $derived(github ? 'ayni.connection.pending' : `ayni.connection.${capability}.pending`);
  const validationMethod = $derived(github ? 'ValidateGithubConnection' : 'ValidateProviderConnection');
  let connections = $state.raw<Connection[]>([]);
  let connection = $state.raw<Connection | null>(null);
  let history = $state.raw<Operation[]>([]);
  let next = $state('');
  let historyNext = $state('');
  let pendingOperationId = $state('');
  let pendingKey = $state('');
  let name = $state('');
  let appId = $state('');
  let pem = $state('');
  let destination = $state('');
  let pushUsername = $state('');
  let pullUsername = $state('');
  let pushToken = $state('');
  let pullToken = $state('');
  let apiKey = $state('');
  let addOpen = $state(false);
  let deleteOpen = $state(false);
  let busy = $state(true);
  let refreshing = $state(false);
  let checking = $state(false);
  let submittingName = $state('');
  let validatingId = $state('');
  let error = $state('');
  const credentialsEntered = $derived(!!(pem || pushToken || pullToken || apiKey));
  const dirty = $derived(connection
    ? name !== connection.displayName || credentialsEntered
    : addOpen && !!(name || appId || destination || pushUsername || pullUsername || credentialsEntered));
  protectUnsavedChanges(() => dirty && !pendingKey);

  function persistKey(key: string) {
    pendingKey = key;
    if (key) sessionStorage.setItem(storageKey, key);
    else sessionStorage.removeItem(storageKey);
  }

  function clearCredentials() {
    pem = '';
    pushToken = '';
    pullToken = '';
    apiKey = '';
  }

  function openAdd() {
    name = '';
    appId = '';
    destination = '';
    pushUsername = '';
    pullUsername = '';
    clearCredentials();
    error = '';
    addOpen = true;
  }

  function showConnection(item: Connection) {
    connections = [item, ...connections.filter(current => current.id !== item.id)];
  }

  async function loadConnections(append = false, includeId = '') {
    const result = await rpc('ListConnections', { afterId: append ? next : '', limit: 50 });
    const sources: Connection[] = (result.connections ?? []).filter((item: Connection) => item.capability === capability);
    if (append) {
      const existing = new Set(connections.map(item => item.id));
      connections = [...connections, ...sources.filter(item => !existing.has(item.id))];
    } else {
      if (includeId && !sources.some(item => item.id === includeId)) {
        try {
          const { connection: included } = await rpc('GetConnection', { connectionId: includeId });
          if (included.capability === capability) sources.unshift(included);
        } catch (cause) {
          if (!(cause instanceof RpcError && cause.status === 404)) throw cause;
        }
      }
      connections = sources;
    }
    next = result.nextAfterId ?? '';
  }

  async function loadHistory(id: string, append = false) {
    const result = await rpc('ListConnectionOperations', { connectionId: id, afterId: append ? historyNext : '', limit: 50 });
    const operations = result.operations ?? [];
    history = append ? [...history, ...operations] : operations;
    historyNext = result.nextAfterId ?? '';
  }

  async function loadDetails(id: string, resetForm: boolean) {
    const result = await rpc('GetConnection', { connectionId: id }).catch(cause => {
      if (cause instanceof RpcError && cause.status === 404) connection = null;
      throw cause;
    });
    const current: Connection = result.connection;
    if (current.capability !== capability) {
      connection = null;
      throw new Error('This connection belongs to a different configuration page.');
    }
    if (resetForm || !dirty) {
      name = current.displayName;
      appId = current.appId;
      destination = current.registryDestination ?? '';
      pushUsername = current.pushUsername ?? '';
      pullUsername = current.pullUsername ?? '';
      clearCredentials();
    }
    connection = current;
    pendingOperationId = result.pendingOperationId ?? '';
    await loadHistory(id);
  }

  async function refresh(resetForm = false, includeId = '') {
    refreshing = true;
    try {
      if (connectionId) await loadDetails(connectionId, resetForm);
      else await loadConnections(false, includeId);
    } finally {
      refreshing = false;
    }
  }

  async function perform(action: () => Promise<void>) {
    busy = true;
    error = '';
    try {
      await action();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : 'Service unavailable. Try again.';
    } finally {
      busy = false;
    }
  }

  function notifyValidation(operation: Operation) {
    const description = reasonText[operation.safeReason ?? ''];
    switch (operation.state) {
      case 'completed': toast.success(github ? 'GitHub App verified.' : 'Credentials validated.'); break;
      case 'failed': toast.warning(github ? 'GitHub App validation failed.' : 'Credential validation failed.', { description }); break;
      case 'superseded': toast.warning('Connection changed before validation finished.'); break;
      default: toast.info('Validation is pending. Retry from connection settings.', { description });
    }
  }

  async function validateCandidate(operationId: string) {
    const { operation } = await rpc(validationMethod, { operationId });
    notifyValidation(operation);
    return operation as Operation;
  }

  async function useConnection(id: string) {
    const result = await rpc('GetConnection', { connectionId: id });
    let current: Connection = result.connection;
    if (result.pendingOperationId) {
      const validated = await validateCandidate(result.pendingOperationId);
      await refresh();
      if (validated.state !== 'completed') return;
      ({ connection: current } = await rpc('GetConnection', { connectionId: id }));
    }
    if (current.enabled && current.activeCredentialRevision) onSaved?.(current);
  }

  async function complete(operation: Operation) {
    persistKey('');
    addOpen = false;
    deleteOpen = false;

    if (operation.kind === 'delete') {
      toast.success('Connection deleted.');
      if (connectionId) {
        connection = null;
        await goto(indexPath);
      } else {
        await refresh();
      }
      return;
    }

    try {
      if (operation.kind === 'enable') {
        toast.success(operation.connection.enabled ? 'Connection enabled.' : 'Connection disabled.');
      } else if (operation.state === 'accepted' && operation.connection.enabled) {
        if (!connectionId) {
          validatingId = operation.connectionId;
          showConnection(operation.connection);
          submittingName = '';
        }
        try {
          operation = await validateCandidate(operation.id);
          if (!connectionId) showConnection(operation.connection);
        } catch {
          throw new Error('Credentials were saved. Open connection settings to retry validation.');
        } finally {
          validatingId = '';
        }
      } else {
        notifyValidation(operation);
      }
    } finally {
      await refresh(operation.kind === 'save', operation.kind === 'save' ? operation.connectionId : '');
    }
    if (onSaved && operation.kind === 'save' && operation.state === 'completed') {
      const { connection: current } = await rpc('GetConnection', { connectionId: operation.connectionId });
      if (current.enabled && current.activeCredentialRevision === operation.connection.activeCredentialRevision) onSaved(current);
    }
  }

  async function submitCommand(method: string, request: object) {
    const key = crypto.randomUUID();
    persistKey(key);
    try {
      const result = await rpc(method, { ...request, idempotencyKey: key });
      return result.operation as Operation;
    } catch (cause) {
      if (cause instanceof RpcError && cause.status === 400) persistKey('');
      throw cause;
    }
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (busy || pendingKey) return;
    const base = {
      connectionId: connection?.id ?? '',
      expectedRevision: connection?.revision ?? '0',
      displayName: name,
    };
    const request = github
      ? { ...base, appId, privateKeyPem: pem }
      : {
        ...base, provider: registry ? 'github' : 'openai', capability,
        registryDestination: destination, pushUsername, pullUsername, pushToken, pullToken, apiKey,
      };
    clearCredentials();
    const adding = !connectionId;
    if (adding) {
      submittingName = base.displayName;
      addOpen = false;
    }
    await perform(async () => {
      try {
        const saved = submitCommand(github ? 'SaveGithubConnection' : 'SaveProviderConnection', request);
        Object.assign(request, { privateKeyPem: '', pushToken: '', pullToken: '', apiKey: '' });
        await complete(await saved);
      } catch (cause) {
        if (adding && cause instanceof RpcError && cause.status === 400) addOpen = true;
        throw cause;
      } finally {
        submittingName = '';
      }
    });
  }

  async function recover() {
    await perform(async () => {
      const result = await rpc('GetConnectionOperation', { idempotencyKey: pendingKey });
      const operation: Operation = result.operation;
      if (operation.connection.capability !== capability || (connectionId && operation.connectionId !== connectionId)) {
        throw new Error('This command belongs to another connection. Recover it from its configuration list.');
      }
      await complete(operation);
    });
  }

  async function checkCredentials(current: Connection) {
    await perform(async () => {
      checking = true;
      try {
        if (pendingOperationId) {
          await validateCandidate(pendingOperationId);
          await refresh();
        } else {
          const { safeReason } = await rpc('CheckConnection', {
            connectionId: current.id, expectedRevision: current.revision,
          });
          if (safeReason) toast.warning(github ? 'GitHub App validation failed.' : 'Credential validation failed.', { description: reasonText[safeReason] });
          else toast.success(github ? 'GitHub App verified.' : 'Credentials validated.');
        }
      } finally {
        checking = false;
      }
    });
  }

  async function changeState(current: Connection, remove = false) {
    await perform(async () => {
      const request = { connectionId: current.id, expectedRevision: current.revision };
      const operation = remove
        ? await submitCommand('DeleteConnection', request)
        : await submitCommand('SetConnectionEnabled', { ...request, enabled: !current.enabled });
      await complete(operation);
    });
  }

  function connectionIdentity(item: Connection) {
    if (github) return [{ label: 'App ID', value: item.appId }];
    if (!registry) return [{ label: 'Endpoint', value: 'api.openai.com' }];
    return [
      { label: 'Organization destination', value: item.registryDestination ?? '' },
      { label: 'Push account', value: item.pushUsername ?? '' },
      { label: 'Pull account', value: item.pullUsername ?? '' },
    ];
  }

  onMount(() => {
    pendingKey = sessionStorage.getItem(storageKey) ?? '';
    if (createOnMount && !pendingKey) openAdd();
    void perform(() => refresh());
  });
</script>

{#snippet recovery()}
  <InformationCard title="Recover a submitted command">
    <p>The response was interrupted. Check whether the command was saved before submitting again.</p>
    <div class="flex flex-wrap gap-3">
      <Button disabled={busy} onclick={recover}>Recover operation</Button>
      <Button variant="outline" disabled={busy} onclick={() => persistKey('')}>Abandon recovery</Button>
    </div>
  </InformationCard>
{/snippet}

{#snippet credentials(editing: boolean)}
  <div class="space-y-2">
    <Label for="connection-name">Connection name</Label>
    <Input id="connection-name" bind:value={name} maxlength={120} required disabled={busy || !!pendingKey} />
  </div>
  {#if github}
  <div class="space-y-2">
    <Label for="app-id">GitHub App ID</Label>
    <Input id="app-id" bind:value={appId} pattern="[1-9][0-9]*" maxlength={20} required disabled={busy || !!pendingKey || editing} />
  </div>
  <div class="space-y-2">
    <Label for="private-key">{editing ? 'Replacement private key (PEM)' : 'Private key (PEM)'}</Label>
    <Textarea id="private-key" bind:value={pem} rows={6} maxlength={32768} required autocomplete="off" spellcheck={false} disabled={busy || !!pendingKey} class="font-mono text-xs" />
  </div>
  {:else if registry}
   <div class="space-y-2"><Label for="registry-destination">Organization destination</Label><Input id="registry-destination" bind:value={destination} placeholder="ghcr.io/ayni-oss" pattern={'ghcr\\.io/[a-z0-9]([a-z0-9\\-]{0,37}[a-z0-9])?'} title="Enter only the organization path, such as ghcr.io/ayni-oss." required maxlength={47} disabled={busy || !!pendingKey || editing}/><p class="text-xs text-muted-foreground">Enter ghcr.io and the organization. The image package comes from each repository, for example sandbox becomes ghcr.io/ayni-oss/sandbox.</p></div>
   <div class="space-y-2"><Label for="push-username">Push username</Label><Input id="push-username" bind:value={pushUsername} required maxlength={100} disabled={busy || !!pendingKey || editing}/></div>
   <div class="space-y-2"><Label for="push-token">{editing ? 'Replacement push PAT':'Push PAT'}</Label><Input id="push-token" type="password" bind:value={pushToken} required maxlength={4096} autocomplete="new-password" disabled={busy || !!pendingKey}/><p class="text-xs text-muted-foreground">Requires write:packages access to this destination.</p></div>
   <div class="space-y-2"><Label for="pull-username">Pull username</Label><Input id="pull-username" bind:value={pullUsername} required maxlength={100} disabled={busy || !!pendingKey || editing}/></div>
   <div class="space-y-2"><Label for="pull-token">{editing ? 'Replacement pull PAT':'Pull PAT'}</Label><Input id="pull-token" type="password" bind:value={pullToken} required maxlength={4096} autocomplete="new-password" disabled={busy || !!pendingKey}/><p class="text-xs text-muted-foreground">Requires read:packages access. This credential is validated independently.</p></div>
  {:else}<div class="space-y-2"><Label for="api-key">{editing ? 'Replacement API key':'API key'}</Label><Input id="api-key" type="password" bind:value={apiKey} required maxlength={4096} autocomplete="new-password" disabled={busy || !!pendingKey}/></div>{/if}
{/snippet}

{#snippet headerActions()}
    {#if !connectionId}
      <Button size="icon" aria-label={surface.add} title={surface.add} disabled={busy || !!pendingKey} onclick={openAdd}>
        <Plus aria-hidden="true" />
      </Button>
    {/if}
    <Button size="icon" variant="outline" aria-label={connectionId ? 'Refresh settings' : 'Refresh connections'} title={connectionId ? 'Refresh settings' : 'Refresh connections'} disabled={busy} onclick={() => perform(() => refresh())}>
      <RefreshCw class={{ 'motion-safe:animate-spin': refreshing }} aria-hidden="true" />
    </Button>
{/snippet}
{#if createOnMount}
  <div class="flex justify-end gap-2">{@render headerActions()}</div>
{:else}
  <PageHeader eyebrow={connectionId ? surface.label : surface.section} title={connection?.displayName ?? (connectionId ? 'Connection settings' : surface.title)} actions={headerActions} />
{/if}

<PageContent>
  {#if error && !addOpen && !deleteOpen}
    <Feedback kind="error" title="Connection needs attention" message={error} />
  {/if}
  {#if pendingKey && !addOpen && !deleteOpen && !submittingName}
    {@render recovery()}
  {/if}

  {#if connectionId}
    {#if connection}
      {@const current = connection}
      <InformationCard title="Connection details" status={current.status} headingLevel={2}>
        <InformationTable rows={[
          ...connectionIdentity(current),
          { label: 'Credential', value: current.activeCredentialRevision ? 'Configured' : 'No active credential' },
          { label: 'Revision', value: current.revision },
        ]} />
        {#if current.safeReason}
          <Feedback kind="warning" title="Latest validation needs attention" message={reasonText[current.safeReason] ?? current.safeReason} />
        {/if}
        <p class="text-sm text-muted-foreground">{surface.validation}</p>
        <Button variant="outline" disabled={busy || !!pendingKey || !current.enabled || !(pendingOperationId || current.activeCredentialRevision)} onclick={() => checkCredentials(current)}>
          {checking ? 'Validating…' : pendingOperationId ? 'Retry validation' : 'Revalidate credentials'}
        </Button>
      </InformationCard>

      <InformationCard title="Edit connection" headingLevel={2}>
        {#if current.enabled}
          <form onsubmit={save} class="space-y-5">
            {@render credentials(true)}
            <p class="text-sm text-muted-foreground">Saving validates the replacement credentials before they become active.</p>
            <Button type="submit" disabled={busy || !!pendingKey}>Save and validate</Button>
          </form>
        {:else}
          <p class="text-sm text-muted-foreground">Enable the connection to update or validate its credentials.</p>
        {/if}
      </InformationCard>

      {#if current.models?.length}
        <InformationCard title="Discovered models" headingLevel={2}>
          <p class="text-sm text-muted-foreground">Choose from these models in project settings.</p>
          <ul class="max-h-64 overflow-y-auto font-mono text-xs">
            {#each current.models as model (model)}<li>{model}</li>{/each}
          </ul>
        </InformationCard>
      {/if}

      <section class="space-y-4" aria-labelledby="connection-operation-history">
        <h2 id="connection-operation-history" class="text-lg font-semibold">Operation history</h2>
        {#each history as operation (operation.id)}
          <InformationCard title={operation.kind === 'save' ? 'Credential submission' : operation.kind === 'delete' ? 'Connection deletion' : 'Connection state'} status={operation.state}>
            <InformationTable rows={[
              { label: 'Operation', value: operation.id, monospace: true },
              { label: 'Revision', value: operation.connectionRevision },
            ]} />
            {#if operation.safeReason}
              <p class="text-sm">{reasonText[operation.safeReason] ?? operation.safeReason}</p>
            {/if}
          </InformationCard>
        {/each}
        {#if historyNext}
          <Button variant="outline" disabled={busy} onclick={() => perform(() => loadHistory(current.id, true))}>Load more history</Button>
        {/if}
      </section>

      <InformationCard title="Danger zone" danger headingLevel={2}>
        <p class="text-sm text-muted-foreground">Disabling or deleting this connection blocks new work that uses it.</p>
        <div class="flex flex-wrap gap-3">
          <Button variant={current.enabled ? 'destructive' : 'outline'} disabled={busy || !!pendingKey} onclick={() => changeState(current)}>
            {current.enabled ? 'Disable connection' : 'Enable connection'}
          </Button>
          <Button variant="destructive" disabled={busy || !!pendingKey} onclick={() => { error = ''; deleteOpen = true; }}>Delete connection</Button>
        </div>
      </InformationCard>

      <Dialog.Root bind:open={deleteOpen}>
        <Dialog.Content>
          <Dialog.Header>
            <Dialog.Title>Delete {current.displayName}?</Dialog.Title>
            <Dialog.Description>This permanently removes the connection from settings and blocks new use. Orchestrator retains past operation records and their credential revisions.</Dialog.Description>
          </Dialog.Header>
          {#if error}<Feedback kind="error" title="Connection needs attention" message={error} />{/if}
          {#if pendingKey}{@render recovery()}{/if}
          <Dialog.Footer>
            <Button variant="outline" onclick={() => deleteOpen = false}>Cancel</Button>
            <Button variant="destructive" disabled={busy || !!pendingKey} onclick={() => changeState(current, true)}>Delete connection</Button>
          </Dialog.Footer>
        </Dialog.Content>
      </Dialog.Root>
    {:else if busy}
      <p role="status" class="text-sm text-muted-foreground">Loading connection…</p>
    {/if}
  {:else}
    <div class="space-y-4">
      <p class="text-sm text-muted-foreground">{surface.description}</p>
      {#if submittingName}
        <InformationCard title={submittingName} status="validating" headingLevel={2}>
          <p role="status" class="text-sm text-muted-foreground">Submitting credentials for validation…</p>
        </InformationCard>
      {/if}
      {#each connections as item (item.id)}
        {@const href = `${indexPath}/${encodeURIComponent(item.id)}`}
        {@const validating = item.id === validatingId}
        <InformationCard
          title={item.displayName}
          status={validating ? 'validating' : item.status}
          navigationHref={validating ? undefined : href}
          headingLevel={2}
          actions={validating ? [] : [
            ...(createOnMount && onSaved && item.enabled && (item.activeCredentialRevision || item.status === 'pending')
              ? [{ label: item.status === 'pending' ? 'Retry validation' : 'Use connection', disabled: busy || !!pendingKey, onSelect: () => { void perform(() => useConnection(item.id)); } }]
              : []),
            { label: 'Settings', onSelect: () => { void goto(href); } },
          ]}
        >
          <InformationTable rows={[
            ...connectionIdentity(item),
            { label: 'Credential', value: item.activeCredentialRevision ? 'Configured' : 'No active credential' },
            { label: 'Revision', value: item.revision },
          ]} />
        </InformationCard>
      {/each}
      {#if next}
        <Button variant="outline" disabled={busy} onclick={() => perform(() => loadConnections(true))}>Load more connections</Button>
      {/if}
    </div>

    <Dialog.Root bind:open={addOpen} onOpenChange={value => { if (!value) clearCredentials(); }}>
      <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl">
        <Dialog.Header>
          <Dialog.Title>{github ? 'Add a GitHub App' : surface.add}</Dialog.Title>
          <Dialog.Description>{github ? 'Enter credentials from an existing GitHub App.' : registry ? 'Enter separate GitHub personal access tokens (classic) for push and pull access.' : 'Enter an OpenAI API key to discover its available models.'} Orchestrator validates the credentials when you save.</Dialog.Description>
        </Dialog.Header>
        {#if error}<Feedback kind="error" title="Connection needs attention" message={error} />{/if}
        {#if pendingKey && !busy}{@render recovery()}{/if}
        {#if busy}<p role="status">Saving and validating credentials…</p>{/if}
        <form onsubmit={save} class="space-y-5" autocomplete="off">
          {@render credentials(false)}
          <Dialog.Footer>
            <Button variant="outline" onclick={() => addOpen = false}>Cancel</Button>
            <Button type="submit" disabled={busy || !!pendingKey}>Save and validate</Button>
          </Dialog.Footer>
        </form>
      </Dialog.Content>
    </Dialog.Root>
  {/if}
</PageContent>
