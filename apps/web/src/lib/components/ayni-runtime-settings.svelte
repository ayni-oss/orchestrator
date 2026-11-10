<script lang="ts">
 import { onMount } from 'svelte';
 import { RefreshCw } from '@lucide/svelte';
 import { toast } from 'svelte-sonner';
 import { rpc, type Connection } from '$lib/api';
 import { runtimeDefaultKey, type RuntimeIntegration, type RuntimeSigner } from '$lib/runtime-preparation';
 import AutosaveField from '$lib/components/autosave-field.svelte';
 import InformationCard from '$lib/components/information-card.svelte';
 import InformationTable from '$lib/components/information-table.svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import * as NativeSelect from '$lib/components/ui/native-select/index.js';

 let { signers, loadingSigners }: { signers: RuntimeSigner[]; loadingSigners: boolean } = $props();
 type Profile = { enabled?: boolean; developmentOnly?: boolean; platform?: string; executorImage?: string };
 let profile = $state.raw<Profile>();
 let registries = $state.raw<Connection[]>([]);
 let loading = $state(true), registryError = $state(false);
 let defaults = $state({ registry: '', signer: '' });
 let revisions = $state({ registry: 0, signer: 0 });
 const availableSigners = $derived(signers.filter(signer => signer.enabled));
 const status = $derived(loading && !profile ? 'Checking' : !profile ? 'Unavailable' :
  !profile.enabled ? 'Disabled' : profile.developmentOnly ? 'Development only' : 'Enabled');

 async function loadRegistries() {
  const connections: Connection[] = [];
  let afterId = '';
  for (let page = 0; page < 20; page++) {
   const result = await rpc('ListConnections', { afterId, limit: 100 });
   connections.push(...((result.connections || []) as Connection[]).filter(connection =>
    connection.capability === 'container_registry' && connection.enabled && connection.activeCredentialRevision));
   afterId = result.nextAfterId || '';
   if (!afterId) break;
  }
  registries = connections;
 }
 async function refresh() {
  loading = true;
  registryError = false;
  const [runtime, registry] = await Promise.allSettled([rpc('GetRuntimeProfile', {}), loadRegistries()]);
  if (runtime.status === 'fulfilled') profile = runtime.value;
  else if (profile) toast.error('Could not refresh runtime status', {description:'The last loaded profile is still shown. Try refreshing again.'});
  registryError = registry.status === 'rejected';
  loading = false;
 }
 function save(kind: RuntimeIntegration, event: Event & { currentTarget: HTMLSelectElement }) {
  const select = event.currentTarget;
  try {
   localStorage.setItem(runtimeDefaultKey(kind), select.value);
   defaults[kind] = select.value;
   revisions[kind] += 1;
  } catch {
   select.value = defaults[kind];
   toast.error('Could not save the runtime default', { description: 'Check that this browser allows site storage, then try again.' });
  }
 }

 onMount(() => {
  try {
   defaults = { registry: localStorage.getItem(runtimeDefaultKey('registry')) || '',
    signer: localStorage.getItem(runtimeDefaultKey('signer')) || '' };
  } catch {
   // The controls remain usable when browser storage cannot be read.
  }
  void refresh();
 });
</script>

{#snippet refreshAction()}
 <Button size="icon" variant="ghost" aria-label="Refresh runtime status" title="Refresh runtime status" disabled={loading} onclick={refresh}>
  <RefreshCw class={{ 'motion-safe:animate-spin': loading }} aria-hidden="true" />
 </Button>
{/snippet}

<InformationCard title="Runtime worker profile" headingLevel={2} {status} headerAction={refreshAction}>
 {#if loading && !profile}
  <InformationTable aligned loading rows={[{label:'Platform',value:''},{label:'Executor image',value:''}]} />
 {:else if !profile}
  <p class="text-sm">Runtime profile status could not be loaded. Refresh to try again.</p>
 {:else if profile.enabled}
  <InformationTable aligned rows={[
   { label: 'Platform', value: profile.platform || 'Unavailable' },
   { label: 'Executor image', value: profile.executorImage || 'Unavailable', monospace: true },
  ]} />
  {#if profile.developmentOnly}
   <p class="text-sm text-muted-foreground">Cluster network isolation is not qualified. Use this profile for local development testing.</p>
  {/if}
 {:else}
  <p class="text-sm">Runtime builds are disabled. A deployment administrator must enable a Kubernetes worker profile with pinned worker, executor, and verifier images.</p>
 {/if}
</InformationCard>

<InformationCard title="Runtime defaults" headingLevel={2}>
 <p class="text-sm text-muted-foreground">These defaults apply in this browser when a repository has no saved selection. Review both choices before starting a build.</p>
 <div class="grid gap-4 sm:grid-cols-2">
  <div class="min-w-0 space-y-2">
   <Label for="ayni-default-registry">Default registry</Label>
   <AutosaveField savedRevision={revisions.registry}>
    <NativeSelect.Root data-slot="input-group-control" id="ayni-default-registry" class="w-full" value={defaults.registry}
     disabled={loading || registryError} onchange={event => save('registry', event)}>
     <NativeSelect.Option value="">No preference</NativeSelect.Option>
     {#if defaults.registry && !registries.some(registry => registry.id === defaults.registry)}
      <NativeSelect.Option value={defaults.registry} disabled>Saved registry unavailable</NativeSelect.Option>
     {/if}
     {#each registries as registry (registry.id)}
      <NativeSelect.Option value={registry.id}>{registry.displayName} · {registry.registryDestination}</NativeSelect.Option>
     {/each}
    </NativeSelect.Root>
   </AutosaveField>
   {#if registryError}<p class="text-sm text-muted-foreground">Registry connections could not be loaded.</p>{/if}
   <Button variant="link" class="px-0" href="/settings/container-registries">Manage registry connections</Button>
  </div>
  <div class="min-w-0 space-y-2">
   <Label for="ayni-default-signer">Default signing key</Label>
   <AutosaveField savedRevision={revisions.signer}>
    <NativeSelect.Root data-slot="input-group-control" id="ayni-default-signer" class="w-full" value={defaults.signer}
     disabled={loadingSigners} onchange={event => save('signer', event)}>
     <NativeSelect.Option value="">No preference</NativeSelect.Option>
     {#if defaults.signer && !availableSigners.some(signer => signer.id === defaults.signer)}
      <NativeSelect.Option value={defaults.signer} disabled>Saved signing key unavailable</NativeSelect.Option>
     {/if}
     {#each availableSigners as signer (signer.id)}
      <NativeSelect.Option value={signer.id}>{signer.keyId}</NativeSelect.Option>
     {/each}
    </NativeSelect.Root>
   </AutosaveField>
  </div>
 </div>
</InformationCard>
