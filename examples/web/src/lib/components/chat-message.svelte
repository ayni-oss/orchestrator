<script lang="ts">
  import * as Message from '$lib/components/ui/message/index.js';
  import * as Bubble from '$lib/components/ui/bubble/index.js';
  import { formatDate, words } from '$lib/presentation.js';
  let { id, role, content, createdAt }: { id: string; role: string; content: string; createdAt?: string } = $props();
  const fromUser = $derived(role === 'user');
</script>
<div data-message-id={id} class="min-w-0">
  <Message.Root align={fromUser ? 'end' : 'start'}>
    <Message.Content>
      <Message.Header class={fromUser ? 'self-end' : ''}>
        <span>{fromUser ? 'You' : role === 'assistant' ? 'Project agent' : words(role)}</span>
        {#if createdAt}<time datetime={createdAt} class="ml-2 font-normal">{formatDate(createdAt)}</time>{/if}
      </Message.Header>
      {#if fromUser}
        <Bubble.Root align="end" variant="muted"><Bubble.Content class="whitespace-pre-wrap">{content}</Bubble.Content></Bubble.Root>
      {:else}
        <Bubble.Root variant="ghost"><Bubble.Content class="whitespace-pre-wrap leading-relaxed">{content}</Bubble.Content></Bubble.Root>
      {/if}
    </Message.Content>
  </Message.Root>
</div>
