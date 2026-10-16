<script lang="ts">
  import { formatDate } from "$lib/presentation.js";
  let { events, attemptNumber }: { events: Array<{sequence:number;message:string;failed:boolean;observedAt?:string;details?:string;truncated?:boolean}>; attemptNumber: number } = $props();
</script>

<details class="rounded-lg border p-3">
  <summary class="cursor-pointer text-sm font-medium">Worker activity · {events.length} events</summary>
  <p class="my-3 text-xs text-muted-foreground">Agent messages, tool calls, results, errors, and provider reasoning summaries appear when available. Private reasoning is unavailable. Credentials are redacted.</p>
  {#if (events[0]?.sequence ?? 0) > 1}<p class="mb-3 text-xs text-muted-foreground">Showing recent activity. New workers retain earlier entries in the worker diagnostic log.</p>{/if}
  <ol class="max-h-96 space-y-3 overflow-auto text-sm" aria-label={`Activity for attempt ${attemptNumber}`}>
    {#each events as event (event.sequence)}
      <li class="border-l-2 pl-3">
        <p class={event.failed ? "text-destructive" : ""}>{event.message}{event.failed ? " — failed" : ""}</p>
        <time class="text-xs text-muted-foreground" datetime={event.observedAt}>{event.observedAt ? formatDate(event.observedAt) : "Time unavailable"}</time>
        {#if event.details}
          <details class="mt-2">
            <summary class="cursor-pointer">View interaction details</summary>
            <pre class="mt-2 max-h-96 overflow-auto whitespace-pre-wrap break-words rounded bg-muted p-3 text-xs">{event.details}</pre>
          </details>
        {/if}
        {#if event.truncated}<p class="text-xs text-muted-foreground">Details truncated; review the retained worker diagnostic log for the full entry.</p>{/if}
      </li>
    {:else}<li class="text-muted-foreground">No worker activity has been retained for this attempt yet.</li>{/each}
  </ol>
</details>
