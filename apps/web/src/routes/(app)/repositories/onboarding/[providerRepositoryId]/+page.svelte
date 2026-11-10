<script lang="ts">
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  import { rpc, RpcError } from '$lib/api';
  import Feedback from '$lib/components/feedback.svelte';
  import InformationCard from '$lib/components/information-card.svelte';
  import InformationTable from '$lib/components/information-table.svelte';
  import PageContent from '$lib/components/page-content.svelte';
  import PageHeader from '$lib/components/page-header.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { repositoryPath } from '$lib/paths';
  import { connectionAttemptError, connectionAttemptResult, readConnectionAttempts, writeConnectionAttempt, type ConnectionAttempt } from '$lib/repository-onboarding';
  import type { RepositoryOperation } from '$lib/repositories';

  let { params } = $props();
  let attempt = $state.raw<ConnectionAttempt | null>(null);
  let loaded = $state(false);
  let busy = $state(false);
  let connecting = $state(false);
  let error = $state('');

  function retain(value: ConnectionAttempt | null) {
    writeConnectionAttempt(params.providerRepositoryId, value);
    attempt = value;
  }

  async function check(retry = false) {
    if (!attempt || busy) return;
    const current = attempt;
    error = '';
    busy = true;
    connecting = retry;
    try {
      const { operation }: { operation: RepositoryOperation } = retry
        ? await rpc('ConnectRepository', current.request)
        : await rpc('GetRepositoryOperation', { idempotencyKey: current.request.idempotencyKey });
      retain(connectionAttemptResult(current, operation));
      if (operation.state === 'completed' && operation.repository) {
        await goto(`${repositoryPath(operation.repository.id)}?tab=runtime`);
      }
    } catch (cause) {
      if (retry) retain(connectionAttemptError(current, cause));
      else error = cause instanceof RpcError && cause.status === 404
        ? 'No saved receipt was found. Retry the same connection to keep the original selection.'
        : 'The saved status could not be checked. Try again.';
    } finally {
      busy = false;
      connecting = false;
    }
  }

  async function dismiss() {
    retain(null);
    await goto('/repositories');
  }

  onMount(() => {
    const attempts = readConnectionAttempts();
    const id = params.providerRepositoryId;
    attempt = Object.hasOwn(attempts, id) ? attempts[id] ?? null : null;
    loaded = true;
  });
</script>

<PageHeader eyebrow="Repository settings" title={attempt?.repository?.fullName ?? 'Repository connection'} />
<PageContent>
  {#if error}<Feedback kind="error" title="Connection status unavailable" message={error} />{/if}
  {#if attempt}
    <InformationCard title="Source code" headingLevel={2} status={connecting ? 'connecting' : attempt.status === 'failed' ? 'connection_failed' : 'pending'}>
      {#if attempt.message}<p class="text-sm">{attempt.message}</p>{/if}
      {#if attempt.repository}
        <InformationTable rows={[
          { label: 'Visibility', value: attempt.repository.private ? 'Private' : 'Public' },
          { label: 'Default branch', value: attempt.repository.defaultBranch },
        ]} />
      {/if}
      {#if attempt.status === 'pending'}
        <p class="text-sm text-muted-foreground">Check the saved status or retry with the same source and repository selection.</p>
        <div class="flex flex-wrap gap-3">
          <Button disabled={busy} onclick={() => check()}>Recover status</Button>
          <Button variant="outline" disabled={busy} onclick={() => check(true)}>Retry same connection</Button>
          <Button variant="outline" disabled={busy} onclick={dismiss}>Dismiss recovery</Button>
        </div>
      {:else}
        <p class="text-sm text-muted-foreground">Review the source connection, then return to Repositories to connect again.</p>
      {/if}
      <Button variant="link" class="px-0" href={`/settings/source-control/github/${encodeURIComponent(attempt.request.connectionId)}`}>Source connection settings</Button>
    </InformationCard>
  {:else if loaded}
    <p class="text-sm text-muted-foreground">Open a repository from the catalog to view its settings.</p>
  {/if}
  <Button variant="outline" href="/repositories">Back to repositories</Button>
</PageContent>
