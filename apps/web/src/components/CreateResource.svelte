<script>
  let { kind, client, view, onclose } = $props();
  let name = $state('');
  let goal = $state('');
  let dialog;
  const labels = { workspace: 'workspace', project: 'project', conversation: 'conversation' };
  $effect(() => { dialog?.showModal(); });
  async function create(event) {
    event.preventDefault();
    const body = kind === 'workspace' ? { displayName: name }
      : kind === 'project' ? { workspaceId: view.scope.workspaceId, displayName: name, goal }
      : { workspaceId: view.scope.workspaceId, projectId: view.scope.projectId, title: name };
    const method = { workspace: 'CreateWorkspace', project: 'CreateProject', conversation: 'CreateConversation' }[kind];
    if (await client.submit(method, body)) onclose();
  }
</script>
<dialog bind:this={dialog} onclose={onclose} aria-labelledby="create-title">
  <form onsubmit={create}>
    <div class="section-row"><h2 id="create-title">New {labels[kind]}</h2><button type="button" class="icon-button" onclick={onclose} aria-label="Close">×</button></div>
    <p class="muted">{kind === 'project' ? 'Give this project a clear goal. Conversations and approved work stay together here.' : kind === 'conversation' ? 'Start a separate discussion with its own agent context.' : 'A shared home for your projects and agent configuration.'}</p>
    <label for="resource-name">{kind === 'conversation' ? 'Title' : 'Name'}</label>
    <input id="resource-name" bind:value={name} required maxlength="256" disabled={view.sending || !!view.pending}>
    {#if kind === 'project'}<label for="resource-goal">Project goal <span class="muted">optional</span></label><textarea id="resource-goal" bind:value={goal} maxlength="16384" rows="3" disabled={view.sending || !!view.pending}></textarea>{/if}
    {#if view.error}<p class="error" role="alert">{view.error}</p>{/if}
    {#if view.pending}<p class="muted">A command needs confirmation. Close this dialog and use the pending-command controls.</p>{/if}
    <div class="dialog-actions"><button type="button" onclick={onclose}>Cancel</button><button class="primary" disabled={view.sending || !!view.pending || view.busy}>Create {labels[kind]}</button></div>
  </form>
</dialog>
