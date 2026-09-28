<script lang="ts">
  import { enhance } from "$app/forms";
  import { ArrowRight, FolderGit2, Plus } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import EmptyState from "$lib/components/empty-state.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import SubmitButton from "$lib/components/submit-button.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { preparationTone } from "$lib/presentation.js";

  let { data, form }: PageProps = $props();
  let enrolling = $state(false);
  const nextPageHref = $derived(data.nextPageToken === undefined ? undefined : `/repositories?pageToken=${encodeURIComponent(data.nextPageToken)}`);
</script>

<svelte:head><title>Repositories · Ayni Orchestrator</title></svelte:head>

<div class="space-y-8" data-bff-status={data.backend}>
  <PageHeader eyebrow="Readiness" title="Repositories" description="Enroll an approved repository, prepare one immutable revision, and verify its retained environment before development starts.">
    {#snippet actions()}<Button href="#enroll"><Plus aria-hidden="true" /> Enroll repository</Button>{/snippet}
  </PageHeader>

  {#if data.error}<Feedback kind="error" title="Repositories unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if form?.error}<Feedback kind="error" title="Enrollment failed" message={form.error.message} requestId={form.error.requestId} />{:else if form?.success}<Feedback kind="success" title="Repository enrolled" message={form.success} />{/if}

  <section aria-labelledby="repository-list" class="space-y-4">
    <h2 id="repository-list" class="sr-only">Enrolled repositories</h2>
    {#if data.repositories.length === 0}
      <EmptyState title="No repositories enrolled" description="Enrollment grants this local control plane access to one approved GitHub repository." />
    {:else}
      <div class="grid gap-4 lg:grid-cols-2">
        {#each data.repositories as item}
          <Card.Root>
            <Card.Header>
              <div class="flex items-start justify-between gap-3">
                <div class="min-w-0"><Card.Title class="truncate">{item.enrollment.owner}/{item.enrollment.repository}</Card.Title><Card.Description class="mt-1">{item.enrollment.id} · base {item.enrollment.baseBranch}</Card.Description></div>
                {#if item.latestPreparation}<StatusBadge status={item.latestPreparation.status} tone={preparationTone(item.latestPreparation.status)} />{:else}<StatusBadge status="not prepared" />{/if}
              </div>
            </Card.Header>
            <Card.Content class="space-y-3 text-sm">
              <div><span class="text-muted-foreground">Ayni lock</span><code class="mt-1 block truncate rounded bg-muted px-2 py-1 text-xs" title={item.enrollment.lockSha256}>{item.enrollment.lockSha256}</code></div>
              {#if item.latestPreparation}
                <div class="grid grid-cols-2 gap-3"><div><span class="block text-xs text-muted-foreground">Revision</span><code class="text-xs">{item.latestPreparation.revision.slice(0, 12)}</code></div><div><span class="block text-xs text-muted-foreground">Reuse count</span>{item.latestPreparation.reuseCount}</div></div>
              {/if}
            </Card.Content>
            <Card.Footer><Button href={`/repositories/${item.enrollment.id}`} variant="outline">Readiness and history <ArrowRight aria-hidden="true" /></Button></Card.Footer>
          </Card.Root>
        {/each}
      </div>
      {#if nextPageHref}<div class="flex justify-end"><Button href={nextPageHref} variant="outline">Next page</Button></div>{/if}
    {/if}
  </section>

  <Card.Root id="enroll">
    <Card.Header><div class="flex items-center gap-3"><span class="grid size-9 place-items-center rounded-md bg-accent"><FolderGit2 class="size-4" aria-hidden="true" /></span><div><Card.Title>Enroll repository</Card.Title><Card.Description>The backend validates product support and authorization; this form does not infer access.</Card.Description></div></div></Card.Header>
    <Card.Content>
      <form method="POST" action="?/enroll" class="grid gap-5 sm:grid-cols-2" use:enhance={() => { enrolling = true; return async ({ update }) => { enrolling = false; await update(); }; }}>
        <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} />
        <div class="space-y-2"><Label for="repository-id">Operator identifier</Label><Input id="repository-id" name="id" required maxlength={64} pattern="[a-z0-9][a-z0-9._-]*" placeholder="sandbox" aria-describedby="repository-id-help" /><p id="repository-id-help" class="text-xs text-muted-foreground">A stable lowercase slug used in task submissions.</p></div>
        <div class="space-y-2"><Label for="installation-id">GitHub App installation ID</Label><Input id="installation-id" name="installationId" required inputmode="numeric" pattern="[1-9][0-9]*" autocomplete="off" /></div>
        <div class="space-y-2"><Label for="owner">Owner</Label><Input id="owner" name="owner" required maxlength={100} value="ayni-oss" autocomplete="off" /></div>
        <div class="space-y-2"><Label for="repository">Repository</Label><Input id="repository" name="repository" required maxlength={100} value="sandbox" autocomplete="off" /></div>
        <div class="space-y-2"><Label for="base-branch">Base branch</Label><Input id="base-branch" name="baseBranch" required maxlength={255} value="main" autocomplete="off" /></div>
        <div class="space-y-2"><Label for="lock-sha">Ayni lock SHA-256</Label><Input id="lock-sha" name="lockSha256" required minlength={64} maxlength={64} pattern={"[0-9a-f]{64}"} autocomplete="off" spellcheck="false" class="font-mono" /></div>
        <div class="sm:col-span-2"><SubmitButton pending={enrolling} pendingLabel="Enrolling…">Enroll repository</SubmitButton></div>
      </form>
    </Card.Content>
  </Card.Root>
</div>
