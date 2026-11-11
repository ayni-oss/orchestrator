<script lang="ts">
  import { onMount } from 'svelte';
  type DevStatus = { checkout: string; branch: string; phase: string; message: string; gateway: string };
  let status = $state.raw<DevStatus | null>(null);
  let connected = $state(false);
  onMount(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
      try {
        const response = await fetch('/__dev/status', { cache: 'no-store', signal: controller.signal });
        if (!response.ok) throw new Error('Development status unavailable');
        status = await response.json();
        connected = true;
      } catch {
        connected = false;
      } finally {
        if (!controller.signal.aborted) timer = setTimeout(refresh, 2000);
      }
    }
    void refresh();
    return () => { controller.abort(); clearTimeout(timer); };
  });
</script>

<aside aria-label="Development environment" class="dev-status">
  <details>
    <summary>Dev · {status?.branch || 'connecting'} · {connected ? status?.phase : 'controller disconnected'}</summary>
    <div>
      <p>{status?.checkout || 'Waiting for cargo xtask dev'}</p>
      <p>{status?.message || 'One development environment at http://127.0.0.1:5174'}</p>
      {#if status?.gateway}<p>{status.gateway}</p>{/if}
    </div>
  </details>
</aside>

<style>
  .dev-status { margin: .5rem 1rem 0; font-size: .75rem; color: #e2e8f0; background: #17202f; border: 1px solid #475569; border-radius: .4rem; padding: .4rem .65rem; overflow-wrap: anywhere; }
  summary { cursor: pointer; }
  p { margin: .4rem 0; overflow-wrap: anywhere; }
</style>
