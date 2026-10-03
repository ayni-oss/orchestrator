<script lang="ts">
  import type { PageProps } from "./$types";
  import CodeValue from "$lib/components/code-value.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import { words } from "$lib/presentation.js";
  let { data }: PageProps = $props();
</script>

<svelte:head><title>Reviews · Ayni Orchestrator</title></svelte:head>
<PageHeader eyebrow="Review" title="Pull requests and assessments" description="Exact-head evidence is authoritative. Comments and checks are summaries of that evidence." />
{#if data.error}<Feedback kind="error" title="Reviews unavailable" message={data.error.message} />{/if}
<div class="space-y-4">
  {#each data.reviews as review (review.id)}
    <section class="space-y-3 rounded-lg border p-4">
      <h2 class="font-semibold"><a href={`?page=${encodeURIComponent(data.pageToken ?? "")}&review=${encodeURIComponent(review.id)}`}>{review.id}</a> · {words(review.state)}</h2>
      <CodeValue value={review.head} />
      {#if review.url}<a href={review.url} rel="noreferrer">Open pull request</a>{/if}
      {#if review.readiness}
        <p>Policy: {words(review.readiness.policy)}. Readiness: {words(review.readiness.state)}.</p>
        <p>Source: {review.readiness.source}. {words(review.readiness.diagnostic)}</p>
        {#if review.readiness.authorizedHead}<p>Authorized head: <CodeValue value={review.readiness.authorizedHead} /></p>{/if}
      {:else}<p>Draft-only policy; readiness has not been evaluated.</p>{/if}
    </section>
  {:else}<p>No assessed pull requests yet.</p>{/each}
</div>
{#if data.page?.nextPageToken}<a href={`?page=${encodeURIComponent(data.page.nextPageToken)}`}>More pull requests</a>{/if}
{#if data.review && data.history}
  <section class="space-y-4">
    <h2 class="text-xl font-semibold">Assessment history</h2>
    {#each data.history.assessments as assessment (assessment.id)}
      <article class="space-y-2 rounded-lg border p-4">
        <h3>{words(assessment.conclusion)} · {assessment.head === data.review.head ? "current head" : "historical head"}</h3>
        <CodeValue value={assessment.head} />
        {#if assessment.managed}<p>{assessment.managed.completed} targets completed; {assessment.managed.skipped} skipped. {assessment.managed.warnings} warnings; {assessment.managed.failures} failures.</p>{/if}
        {#if assessment.workflowUrl}<a href={assessment.workflowUrl} rel="noreferrer">Workflow and retained findings</a>{/if}
        <a class="ml-4" href={`?page=${encodeURIComponent(data.pageToken ?? "")}&history=${encodeURIComponent(data.historyToken ?? "")}&review=${encodeURIComponent(data.review.id)}&assessment=${encodeURIComponent(assessment.id)}`}>Evidence identity</a>
      </article>
    {/each}
    {#if data.history.page.nextPageToken}<a href={`?page=${encodeURIComponent(data.pageToken ?? "")}&review=${encodeURIComponent(data.review.id)}&history=${encodeURIComponent(data.history.page.nextPageToken)}`}>More assessments</a>{/if}
    {#each data.evidence as evidence (evidence.id)}
      <div class="rounded border p-3"><p>{evidence.name} · {evidence.sizeBytes} bytes</p><CodeValue value={evidence.digest} /></div>
    {/each}
  </section>
{/if}
