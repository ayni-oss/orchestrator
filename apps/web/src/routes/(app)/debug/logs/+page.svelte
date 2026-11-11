<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/state';
  import { rpc } from '$lib/api';
  import { decodeLogEntries, appendLogPage, type LogSource, type LogEntry, type LogPage } from '$lib/debug-logs';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import PageHeader from '$lib/components/page-header.svelte';

  let sources = $state.raw<LogSource[]>([]);
  let entries = $state.raw<LogEntry[]>([]);
  let sourceId = $state('');
  let component = $state('');
  let executionId = $state(page.url.searchParams.get('executionId') ?? '');
  let namespace = $state('');
  let podUid = $state('');
  let container = $state('');
  let restart = $state('');
  let stream = $state('');
  let text = $state('');
  let from = $state('');
  let until = $state('');
  let cursor = $state('0');
  let expired = $state('0');
  let status = $state('Loading sources');
  let error = $state('');
  let following = $state(false);
  let loading = $state(true);
  let discarded = $state(false);
  let sourceCursor = $state('');
  let connection: EventSource | undefined;
  let requestVersion = 0;
  const decoded = $derived(decodeLogEntries(entries));
  const selected = $derived(sources.find(source => source.id === sourceId));
  function query(after = cursor) {
    return { after, sourceId, component, executionId, namespace, podUid, container, ...(restart === '' ? {} : { restart: Number(restart) }), stream, text, fromUnixMs: from ? String(new Date(from).getTime()) : '0', untilUnixMs: until ? String(new Date(until).getTime()) : '0' };
  }
  function parameters(after = cursor) { return new URLSearchParams(Object.entries(query(after)).map(([key, value]) => [key, String(value)])); }
  function accept(value: LogPage) {
    discarded ||= entries.length + (value.entries?.length ?? 0) > 1000;
    entries = appendLogPage(entries, value);
    cursor = value.nextCursor ?? cursor;
    expired = value.expiredThroughCursor ?? expired;
    const known = new Map(sources.map(source => [source.id, source]));
    for (const entry of value.entries ?? []) {
      const source = entry.record?.source;
      if (source && Number(source.observedAtUnixMs ?? 0) >= Number(known.get(source.id)?.observedAtUnixMs ?? 0)) known.set(source.id, source);
    }
    sources = [...known.values()];
  }
  function pause() { connection?.close(); connection = undefined; following = false; status = 'Paused'; }
  function follow() {
    connection?.close(); following = true; status = 'Connecting';
    connection = new EventSource(`/debug/logs/stream?${parameters()}`);
    connection.onopen = () => { status = 'Following'; error = ''; };
    connection.addEventListener('logs', event => { accept(JSON.parse((event as MessageEvent).data)); status = 'Following'; });
    connection.addEventListener('unavailable', () => { status = 'Disconnected · retrying'; });
    connection.onerror = () => { status = 'Disconnected · retrying'; };
  }
  async function loadSources(append = false) {
    try {
      const result = await rpc('ListLogSources', { executionId, afterId: append ? sourceCursor : '' });
      sources = append ? [...sources, ...(result.sources ?? [])] : result.sources ?? [];
      sourceCursor = result.nextAfterId ?? '';
    } catch { error = 'Sources are unavailable. Retry when the local service is ready.'; }
  }
  async function read(reset = false) {
    pause(); const version = ++requestVersion; loading = true; error = '';
    if (reset) { entries = []; cursor = '0'; discarded = false; }
    try {
      const value = await rpc('ReadLogs', query());
      if (version !== requestVersion) return;
      accept(value); status = 'History';
    } catch { if (version === requestVersion) error = 'Logs are unavailable. Retry when the local service is ready.'; }
    finally { if (version === requestVersion) loading = false; }
  }
  async function apply() { pause(); loading = true; await loadSources(); await read(true); }
  async function copy() {
    try { await navigator.clipboard.writeText(entries.map(entry => decoded.get(entry.cursor) ?? '').join('')); }
    catch { error = 'Could not copy the displayed logs.'; }
  }
  onMount(() => {
    void (async () => { await loadSources(); await read(); })();
    return () => { ++requestVersion; connection?.close(); };
  });
</script>

<PageHeader title="Logs" eyebrow="Debug" />
<div class="notice" role="note">Debug mode shows raw output. Kubernetes API capture can miss output during rotation, restarts or pod loss. Records retain source order; timestamps from different machines may differ.</div>
<form class="filters" onsubmit={event => { event.preventDefault(); void apply(); }}>
  <label>Source<Input aria-label="Source ID" list="log-sources" bind:value={sourceId} placeholder="All sources" /></label>
  <datalist id="log-sources">{#each sources as source (source.id)}<option value={source.id}>{source.podName || source.component || source.id} / {source.container || 'lifecycle'} · restart {source.restart ?? 0}</option>{/each}</datalist>
  <label>Component<Input bind:value={component} placeholder="All components" /></label>
  <label>Execution<Input bind:value={executionId} placeholder="All executions" /></label>
  <label>Namespace<Input bind:value={namespace} placeholder="All namespaces" /></label>
  <label>Pod UID<Input bind:value={podUid} placeholder="All pods" /></label>
  <label>Container<Input bind:value={container} placeholder="All containers" /></label>
  <label>Restart<Input type="number" min="0" bind:value={restart} placeholder="All" /></label>
  <label>Stream<Input list="log-streams" bind:value={stream} placeholder="All streams" /></label>
  <datalist id="log-streams"><option>stdout</option><option>stderr</option><option>combined</option></datalist>
  <label>From<Input type="datetime-local" bind:value={from} /></label>
  <label>Until<Input type="datetime-local" bind:value={until} /></label>
  <label class="search">Search output<Input bind:value={text} placeholder="Find text in retained output" /></label>
  <Button type="submit" disabled={loading}>Apply filters</Button>
</form>
<div class="toolbar">
  <Button variant="outline" onclick={() => void read()} disabled={loading || following}>Load next page</Button>
  <Button variant="outline" disabled={loading} onclick={following ? pause : follow}>{following ? 'Pause' : 'Follow live'}</Button>
  <Button variant="outline" onclick={() => void loadSources()}>Refresh sources</Button>
  {#if sourceCursor}<Button variant="outline" onclick={() => void loadSources(true)}>More sources</Button>{/if}
  <Button variant="outline" onclick={() => void copy()} disabled={!entries.length}>Copy displayed</Button>
  <Button variant="outline" href={`/debug/logs/download?${parameters('0')}`}>Download original bytes</Button>
  <span role="status">{status}</span>
</div>
{#if error}<p role="alert">{error}</p>{/if}
{#if expired !== '0'}<p class="notice">Older history has expired. Available history starts after cursor {expired}.</p>{/if}
{#if discarded}<p class="notice">Showing the latest 1,000 loaded records. Download retained history to read more.</p>{/if}
{#if selected}<p class="coverage">{selected.podName} / {selected.container} · {selected.captureStatus ?? 'Completeness unknown'} · {selected.provenance}</p>{/if}
<details>
  <summary>{sources.length} registered sources</summary>
  <div class="sources">{#each sources as source (source.id)}
    <Button variant="ghost" onclick={() => { sourceId = source.id; void read(true); }}>{source.podName || source.component || source.id} / {source.container || 'lifecycle'} · {source.restart ?? 0} · {source.captureStatus ?? 'Completeness unknown'}</Button>
  {/each}</div>
</details>
<div class="output" role="region" aria-label="Execution logs" aria-busy={loading}>
  {#if !entries.length}<p class="empty">{loading ? 'Loading logs…' : 'No output in this page. Load the next page or follow live; check source status for capture availability.'}</p>{/if}
  {#each entries as entry (entry.cursor)}
    {@const record = entry.record}
    <article class:gap={record?.event?.includes('gap') || record?.event?.includes('uncertain')}>
      <div class="metadata"><time>{new Date(Number(record?.observedAtUnixMs ?? '0')).toISOString()}</time><span>{record?.source?.podName || record?.source?.component || record?.source?.id} / {record?.source?.container || 'lifecycle'}</span><span>{record?.stream || record?.event}</span><span>{record?.actionId ?? ''}</span></div>
      {#if record?.data}<pre>{decoded.get(entry.cursor) ?? ''}</pre>{:else}<p>{record?.event}{record?.exitCode !== undefined ? ` · exit ${record.exitCode}` : ''}{record?.signal ? ` · signal ${record.signal}` : ''}{record?.lostBytes ? ` · ${record.lostBytes} bytes lost` : ''}</p>{/if}
    </article>
  {/each}
</div>
<style>
  .notice{padding:12px 16px;border:1px solid var(--border);border-radius:var(--radius);margin:16px 0;color:var(--muted-foreground)}
  .filters{display:grid;grid-template-columns:repeat(auto-fit,minmax(170px,1fr));gap:12px;align-items:end;margin:20px 0}.filters label{display:grid;gap:6px;font-size:13px}.search{grid-column:span 2}
  .toolbar{display:flex;gap:8px;flex-wrap:wrap;align-items:center;margin:16px 0}.toolbar span{color:var(--muted-foreground);font-size:13px}
  .sources{display:flex;flex-direction:column;max-height:260px;overflow:auto;align-items:start}.coverage,details{font-size:13px;margin:12px 0}
  .output{border:1px solid var(--border);border-radius:var(--radius);background:var(--card);height:60vh;overflow:auto;margin-top:16px;font:12px var(--font-mono)}article{border-bottom:1px solid var(--border);padding:8px 12px}.metadata{display:flex;flex-wrap:wrap;gap:12px;font-size:10px;color:var(--muted-foreground)}pre{white-space:pre-wrap;overflow-wrap:anywhere;margin:5px 0}article p{margin:5px 0}.gap{border-left:3px solid var(--destructive)}.empty{padding:20px;color:var(--muted-foreground)}
  @media(max-width:540px){.search{grid-column:auto}.filters{grid-template-columns:1fr}.output{height:55vh}}
</style>
