<script lang="ts">
  import type { PageProps } from './$types';

  let { data, form }: PageProps = $props();
</script>

<svelte:head>
  <title>Ayni Orchestrator browser boundary</title>
  <meta
    name="description"
    content="Server-rendered browser boundary backed by the native Orchestrator gRPC service"
  />
</svelte:head>

<main data-bff-status={data.backend} data-grpc-provider={data.provider ?? 'unavailable'}>
  <h1>Ayni Orchestrator</h1>
  <p>
    This minimal page validates the server-only browser boundary. The maintained operator interface
    is delivered separately by the browser-application work.
  </p>

  <dl>
    <div><dt>Control plane</dt><dd>{data.backend}</dd></div>
    <div><dt>Repositories</dt><dd>{data.repositories.length}</dd></div>
    <div><dt>Provider models</dt><dd>{data.models.length}</dd></div>
    <div><dt>Tasks</dt><dd>{data.tasks.length}</dd></div>
  </dl>

  {#if data.error}
    <p role="status">{data.error.message} ({data.error.reason})</p>
  {/if}

  <form method="POST" action="?/inspectTask">
    <label for="task-id">Inspect one task through a server form action</label>
    <div>
      <input id="task-id" name="taskId" autocomplete="off" required maxlength="36" />
      <button type="submit">Inspect</button>
    </div>
  </form>

  {#if form?.lookupError}
    <p role="alert">{form.lookupError.message}</p>
  {:else if form?.inspectedTask}
    <p role="status">Task {form.inspectedTask.id}: {form.inspectedTask.phase}</p>
  {/if}

  <small>Request ID: {data.requestId}</small>
</main>

<style>
  :global(body) {
    margin: 0;
    font-family: system-ui, sans-serif;
    color: #172033;
    background: #f5f7fa;
  }
  main {
    max-width: 48rem;
    margin: 4rem auto;
    padding: 2rem;
    border: 1px solid #d7dce5;
    border-radius: 0.75rem;
    background: white;
  }
  dl {
    display: grid;
    gap: 0.5rem;
  }
  dl div {
    display: flex;
    justify-content: space-between;
  }
  dt {
    font-weight: 600;
  }
  form {
    margin-block: 2rem;
  }
  form div {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }
  input {
    flex: 1;
  }
  input,
  button {
    min-height: 2.5rem;
    font: inherit;
  }
</style>
