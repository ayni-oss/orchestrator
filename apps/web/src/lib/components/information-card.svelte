<script module lang="ts">
  export type InformationCardAction = {
    label: string;
    onSelect: () => void;
    disabled?: boolean;
    destructive?: boolean;
  };
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import { EllipsisVertical } from "@lucide/svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import { statusTone, type StatusTone } from "$lib/presentation.js";

  let { title, titleAccessory, href, navigationHref, status, tone, actions = [], danger = false, headingLevel = 3, children }: {
    title: string;
    titleAccessory?: Snippet;
    href?: string;
    navigationHref?: string;
    status?: string;
    tone?: StatusTone;
    actions?: InformationCardAction[];
    danger?: boolean;
    headingLevel?: 2 | 3;
    children: Snippet;
  } = $props();
</script>

<Card.Root class={["relative min-w-0", danger && "ring-destructive/40", navigationHref && "transition-colors hover:bg-accent/30"]} role="region" aria-label={title}>
  {#if navigationHref}
    <a href={navigationHref} aria-label={`Open ${title}`} class="absolute inset-0 z-10 rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"></a>
  {/if}
  <Card.Header class="flex items-center justify-between gap-3">
    <Card.Title class="min-w-0 flex-1">
      {#if headingLevel === 2}
        <h2 class="flex items-center gap-2"><span class="min-w-0 break-words">{#if href}<a {href} class="hover:underline">{title}</a>{:else}{title}{/if}</span>{#if titleAccessory}{@render titleAccessory()}{/if}</h2>
      {:else}
        <h3 class="flex items-center gap-2"><span class="min-w-0 break-words">{#if href}<a {href} class="hover:underline">{title}</a>{:else}{title}{/if}</span>{#if titleAccessory}{@render titleAccessory()}{/if}</h3>
      {/if}
    </Card.Title>
    <div class="ml-auto flex shrink-0 items-center gap-3">
      {#if status}<StatusBadge {status} tone={tone ?? statusTone(status)} />{/if}
      {#if actions.length}
        <DropdownMenu.Root>
          <DropdownMenu.Trigger>
            {#snippet child({ props })}
              <Button {...props} class="relative z-20" variant="ghost" size="icon-sm" aria-label={`${title} actions`}><EllipsisVertical aria-hidden="true" /></Button>
            {/snippet}
          </DropdownMenu.Trigger>
          <DropdownMenu.Content align="end" preventScroll={false}>
            {#each actions as action (action.label)}
              <DropdownMenu.Item onSelect={action.onSelect} disabled={action.disabled ?? false} variant={action.destructive ? "destructive" : "default"}>{action.label}</DropdownMenu.Item>
            {/each}
          </DropdownMenu.Content>
        </DropdownMenu.Root>
      {:else}
        <span class="size-7 shrink-0" aria-hidden="true"></span>
      {/if}
    </div>
  </Card.Header>
  <Card.Content class={['space-y-4', navigationHref && '[&_a]:relative [&_a]:z-20 [&_button]:relative [&_button]:z-20']}>{@render children()}</Card.Content>
</Card.Root>
