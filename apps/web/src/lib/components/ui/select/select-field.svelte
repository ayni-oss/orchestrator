<script lang="ts">
  import { Select } from "bits-ui";
  import { Check, ChevronDown } from "@lucide/svelte";
  let { id, name, form, value = $bindable(""), options, placeholder = "Select an option", disabled = false, required = false, onValueChange }: {
    id: string;
    name?: string;
    form?: string;
    value?: string;
    options: { value: string; label: string; disabled?: boolean }[];
    placeholder?: string;
    disabled?: boolean;
    required?: boolean;
    onValueChange?: (value: string) => void;
  } = $props();
  const label = $derived(options.find(option => option.value === value)?.label);
</script>

<Select.Root type="single" bind:value {disabled} {required} items={options} {onValueChange}>
  <Select.Trigger {id} aria-required={required} class="border-input bg-background dark:bg-input/30 focus-visible:border-ring focus-visible:ring-ring/50 flex h-9 w-full items-center justify-between gap-2 rounded-lg border px-3 text-sm shadow-xs outline-none focus-visible:ring-3 disabled:cursor-not-allowed disabled:opacity-50">
    <span class={["truncate", !label && "text-muted-foreground"]}>{label ?? placeholder}</span>
    <ChevronDown class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
  </Select.Trigger>
  <Select.Portal>
    <Select.Content sideOffset={4} align="start" class="bg-popover text-popover-foreground border-border z-[60] max-h-[var(--bits-select-content-available-height)] w-[var(--bits-select-anchor-width)] overflow-y-auto rounded-lg border p-1 shadow-md outline-none">
      {#each options as option (option.value)}
        <Select.Item value={option.value} label={option.label} disabled={option.disabled} class="data-highlighted:bg-accent data-highlighted:text-accent-foreground relative flex cursor-default select-none items-center gap-2 rounded-md py-2 pl-2 pr-8 text-sm outline-none data-disabled:pointer-events-none data-disabled:opacity-50">
          {option.label}
          {#if value === option.value}<Check class="absolute right-2 size-4" aria-hidden="true" />{/if}
        </Select.Item>
      {/each}
    </Select.Content>
  </Select.Portal>
</Select.Root>
{#if name}<input type="hidden" {name} {value} {form} {disabled} />{/if}
