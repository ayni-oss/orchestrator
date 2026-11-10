<script lang="ts">
  import Skeleton from "./ui/skeleton/skeleton.svelte";
  import TitleRow from "./title-row.svelte";
  import type { Snippet } from "svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import ActionMenu, { type ActionMenuItem } from "$lib/components/action-menu.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import { statusTone, type StatusTone } from "$lib/presentation.js";

  let { title, titleAccessory, headerAction, href, navigationHref, status, tone, actions: menuActions = [], loadingTitle = false, danger = false, headingLevel = 3, children }: {
    title: string;
    loadingTitle?: boolean;
    titleAccessory?: Snippet;
    headerAction?: Snippet;
    href?: string;
    navigationHref?: string;
    status?: string;
    tone?: StatusTone;
    actions?: ActionMenuItem[];
    danger?: boolean;
    headingLevel?: 2 | 3;
    children: Snippet;
  } = $props();
</script>

<Card.Root class={["relative min-w-0", danger && "ring-destructive/40", navigationHref && "transition-colors hover:bg-accent/30"]} role="region" aria-label={title}>
  {#if navigationHref}
    <a href={navigationHref} aria-label={`Open ${title}`} class="absolute inset-0 z-10 rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"></a>
  {/if}
  <Card.Header>
    <TitleRow>
      {#snippet children()}
        <Card.Title class="min-w-0">
          <svelte:element this={`h${headingLevel}`} class="min-w-0 break-words" aria-busy={loadingTitle}>{#if loadingTitle}<span class="sr-only">{title}</span><Skeleton class="h-5 w-40 max-w-full" />{:else if href}<a {href} class="hover:underline">{title}</a>{:else}{title}{/if}</svelte:element>
        </Card.Title>
      {/snippet}
      {#snippet badge()}
        {#if titleAccessory}{@render titleAccessory()}{/if}
        {#if status}<StatusBadge {status} tone={tone ?? statusTone(status)} />{/if}
      {/snippet}
      {#snippet actions()}
        {#if headerAction}<div class="relative z-20">{@render headerAction()}</div>{/if}
        {#if menuActions.length}<ActionMenu label={`${title} actions`} actions={menuActions} triggerClass="relative z-20" />{/if}
      {/snippet}
    </TitleRow>
  </Card.Header>
  <Card.Content class={['space-y-4', navigationHref && '[&_a]:relative [&_a]:z-20 [&_button]:relative [&_button]:z-20']}>{@render children()}</Card.Content>
</Card.Root>
