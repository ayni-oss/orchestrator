<script lang="ts">
  import { Check, ChevronsUpDown } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Popover from "$lib/components/ui/popover/index.js";
  import * as Checkbox from "$lib/components/ui/checkbox/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  let { repositories, value = $bindable([]), id = "repositories", invalid = false, describedBy, onchange }: {
    repositories: Array<{id:string;displayName:string}>; value?: string[]; id?:string; invalid?:boolean; describedBy?:string | undefined; onchange?:(values:string[])=>void;
  } = $props();
  let open = $state(false), search = $state("");
  const filtered = $derived(repositories.filter(repository => repository.displayName.toLowerCase().includes(search.toLowerCase())));
  function toggle(id:string, checked:boolean) { value = checked ? [...value.filter(member => member !== id), id] : value.filter(member => member !== id); onchange?.(value); }
</script>
<Popover.Root bind:open>
  <Popover.Trigger id={id} class="flex h-10 w-full items-center justify-between rounded-lg border bg-background px-3 text-left text-sm" aria-invalid={invalid} aria-describedby={describedBy}>
    <span>{value.length ? `${value.length} ${value.length === 1 ? "repository" : "repositories"} selected` : "Select repositories"}</span><ChevronsUpDown class="size-4" aria-hidden="true" />
  </Popover.Trigger>
  <Popover.Content class="w-[min(26rem,calc(100vw-3rem))] space-y-3" align="start">
    <Input aria-label="Search repositories" placeholder="Search repositories…" bind:value={search} />
    <div class="max-h-64 space-y-1 overflow-y-auto" role="group" aria-label="Repository choices">
      {#each filtered as repository (repository.id)}
        <label class="flex cursor-pointer items-center gap-3 rounded-md p-2 hover:bg-accent">
          <Checkbox.Root checked={value.includes(repository.id)} onCheckedChange={checked => toggle(repository.id, checked === true)} aria-label={repository.displayName} />
          <span class="break-all text-sm">{repository.displayName}</span>
        </label>
      {:else}<p class="p-2 text-sm text-muted-foreground">No matching repositories.</p>{/each}
    </div>
    <Button type="button" variant="outline" size="sm" onclick={() => open = false}><Check aria-hidden="true" /> Done</Button>
  </Popover.Content>
</Popover.Root>
{#each value as member (member)}<input type="hidden" name="repositoryConnectionIds" value={member} />{/each}
