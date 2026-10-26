<script>
  import { onMount } from 'svelte';
  import { createWorkspace, cleanScope } from './workspace.mjs';
  import { consoleLink } from './api.mjs';
  import Sidebar from './components/Sidebar.svelte';
  import CreateResource from './components/CreateResource.svelte';
  import Conversation from './components/Conversation.svelte';
  import Inspector from './components/Inspector.svelte';
  const query = Object.fromEntries(new URLSearchParams(location.search));
  let storage;
  try { storage = sessionStorage; } catch { /* Retry state stays in memory when storage is unavailable. */ }
  const client = createWorkspace({ storage, EventSource, initialScope: Object.keys(cleanScope(query)).length ? query : undefined });
  let view = $state.raw(client.state);
  let creation = $state('');
  let mobileOpen = $state(false);
  let now = $state(Date.now());
  let dark = $state(false);
  const project = $derived(view.projects.find(item => item.id === view.scope.projectId));
  const workspace = $derived(view.workspaces.find(item => item.id === view.scope.workspaceId));
  const conversation = $derived(view.conversations.find(item => item.id === view.scope.conversationId));
  const handoff = $derived(consoleLink(view.scope));
  onMount(() => {
    const unsubscribe = client.subscribe(next => {
      view = next;
      const selection = new URLSearchParams(next.scope);
      history.replaceState(null, '', `/workspace/${selection.size ? `?${selection}` : ''}`);
    });
    void client.start();
    dark = matchMedia('(prefers-color-scheme: dark)').matches;
    const timer = setInterval(() => { now = Date.now(); }, 1000);
    return () => { clearInterval(timer); unsubscribe(); client.dispose(); };
  });
</script>
<svelte:window onkeydown={event => { if (event.key === 'Escape') mobileOpen = false; }} />
<svelte:head><title>{project ? `${project.displayName} · ` : ''}Ayni Orchestrator</title></svelte:head>
<div class="workspace-shell" class:dark>
  <a class="skip-link" href="#main-content">Skip to content</a>
  <Sidebar {view} {client} bind:mobileOpen oncreate={kind => creation = kind} />
  {#if mobileOpen}<button class="mobile-backdrop" aria-label="Dismiss navigation backdrop" onclick={() => mobileOpen = false}></button>{/if}
  <div class="main-shell">
    <header class="topbar"><button class="icon-button mobile-toggle" aria-label="Toggle navigation" aria-expanded={mobileOpen} onclick={() => mobileOpen = !mobileOpen}>☰</button><div class="breadcrumbs"><span>{workspace?.displayName || 'Workspace'}</span><span aria-hidden="true">/</span><strong>{project?.displayName || 'Projects'}</strong></div><button class="icon-button theme-toggle" aria-label={dark ? 'Use light theme' : 'Use dark theme'} onclick={() => dark = !dark}>{dark ? '☀' : '☾'}</button><a class="console-link" href={handoff}>Contributor console ↗</a></header>
    <main id="main-content">
      <div class="project-heading"><div><p class="eyebrow">PROJECT WORKSPACE</p><h1>{project?.displayName || 'A place to build what’s next.'}</h1><p class="project-goal">{project?.goal || 'Bring intent, conversations and approved work together.'}</p></div>{#if project}<button class="primary" onclick={() => creation = 'conversation'} disabled={view.busy || view.sending || !!view.pending}>New conversation <span aria-hidden="true">+</span></button>{/if}</div>
      {#if view.storageWarning}<p class="notice" role="alert">{view.storageWarning}</p>{/if}
      {#if view.error && !creation}<div class="error-banner" role="alert"><p>{view.error}</p><button class="text-button" onclick={() => client.dismissError()}>Dismiss</button></div>{/if}
      {#if view.pending}<section class="pending-banner" aria-label="Pending command"><strong>Confirm an interrupted command</strong><p>{view.pending.method} is awaiting confirmation. Retry preserves the original content, scope and command key. This tab pauses new commands until the outcome is known.</p><details><summary>Original command and scope</summary><pre>{JSON.stringify(view.pending, null, 2)}</pre></details><button onclick={() => client.retry()} disabled={view.sending}>{view.sending ? 'Confirming…' : 'Retry the same command'}</button></section>{/if}
      {#if !project}
        <section class="welcome"><span class="empty-symbol" aria-hidden="true">✳</span><h2>{view.scope.workspaceId ? 'Give your next idea a home.' : 'Your software factory starts here.'}</h2><p>{view.scope.workspaceId ? 'Create a project with a clear goal, then start a conversation.' : 'Create a workspace to organize projects and isolated agent conversations.'}</p><button class="primary" onclick={() => creation = view.scope.workspaceId ? 'project' : 'workspace'} disabled={view.busy || view.sending || !!view.pending}>{view.scope.workspaceId ? 'Create a project' : 'Create a workspace'}</button></section>
      {:else}
        <div class="project-toolbar"><h2>Conversations</h2><span class="muted">{view.conversations.length} loaded</span><a href={handoff}>Review plans, tasks & recovery ↗</a></div>
        <div class="project-content">
          <nav class="conversation-list" aria-label="Conversations">{#each view.conversations as item (item.id)}<button class:selected={item.id === view.scope.conversationId} aria-current={item.id === view.scope.conversationId ? 'page' : undefined} disabled={view.busy || view.sending} onclick={() => client.select({ ...view.scope, conversationId: item.id })}><span>{item.title}</span><small>{item.createdAt ? new Date(item.createdAt).toLocaleDateString() : 'Conversation'}</small></button>{:else}<p class="muted">Start a conversation to explore the project.</p>{/each}{#if view.pages.conversations}<button class="text-button" onclick={() => client.more('conversations')}>Load more conversations</button>{/if}</nav>
          <div class="conversation-workspace">
            {#if view.scope.conversationId}
              <div class="conversation-heading"><h2>{view.snapshot?.title || conversation?.title || 'Opening conversation…'}</h2><div class="connection"><span class="status-dot" class:live={view.connection === 'Live'}></span><span role="status">{view.connection}</span><button class="text-button" onclick={() => view.connection.startsWith('Paused') ? client.reconnect() : client.pause()}>{view.connection.startsWith('Paused') ? 'Reconnect' : 'Pause updates'}</button></div></div>
              <div class="conversation-columns">{#key view.scope.conversationId}<Conversation {view} {client} {now} />{/key}<Inspector {view} {now} {client} /></div>
            {:else}<section class="welcome"><h2>What would you like to explore?</h2><p>Choose a conversation or start a new discussion.</p></section>{/if}
          </div>
        </div>
      {/if}
    </main>
  </div>
  {#if creation}<CreateResource kind={creation} {client} {view} onclose={() => creation = ''} />{/if}
</div>
