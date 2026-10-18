<script lang="ts">
  import WorkerActivity from "$lib/components/worker-activity.svelte";
  import AgentProfileSummary from "$lib/components/agent-profile-summary.svelte";
  import {projectHref} from "$lib/project-navigation.js";
  import { enhance } from "$lib/forms.js";
  import type { SubmitFunction } from "@sveltejs/kit";
  import type { PageProps } from "./$types";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import TaskUpdateStatus from "$lib/components/task-update-status.svelte";
  import { onMount } from "svelte";
  import { subscribeRefresh } from "$lib/refresh.js";
  import ExecutionOutcomes from "$lib/components/execution-outcomes.svelte";
  import QualitySignal from "$lib/components/quality-signal.svelte";
  import EmptyState from "$lib/components/empty-state.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { formatDate, formatTokens, words } from "$lib/presentation.js";

  let { data }: PageProps = $props();
  const latestAttempt = $derived(data.progress?.attempts.at(-1));
  const canRetryPublication = $derived(Boolean(latestAttempt?.result?.commit && ["failed", "conflict"].includes(latestAttempt.result.publication)));
  let retryForm = $state<HTMLFormElement | undefined>();
  let publicationForm = $state<HTMLFormElement | undefined>();
  let cancelForm = $state<HTMLFormElement | undefined>();
  let busy = $state(false);
  const terminalStates = ["succeeded", "failed", "cancelled", "deadline_exceeded", "unsupported"];
  const submit: SubmitFunction = () => {
    busy = true;
    return async ({ update }) => {
      try { await update(); } finally { busy = false; }
    };
  };
  onMount(() => subscribeRefresh("app:tasks", { intervalMs: 120_000 }));
</script>

<svelte:head><title>{data.task?.snapshot.request.slice(0, 60) ?? "Task unavailable"} · Ayni Orchestrator</title></svelte:head>
<div class="max-w-4xl space-y-6">
  <Button href="/_old/tasks" variant="ghost">Development tasks</Button>
  {#if data.task}
    {@const task = data.task}
    {@const terminal = terminalStates.includes(task.state)}
    <Button href={projectHref(task.projectId,"tasks",task.snapshot.origin?.conversationId)} variant="outline">Project tasks</Button>
    <PageHeader eyebrow="Task" title={task.snapshot.request} />
    <AgentProfileSummary profile={task.snapshot.agentProfile} />
    {#if task.snapshot.origin}
      {@const origin=task.snapshot.origin}
      <nav class="flex flex-wrap gap-3 text-sm" aria-label="Task origin"><a class="underline" href={projectHref(origin.projectId,"overview",origin.conversationId)}>Originating conversation</a><a class="underline" href={projectHref(origin.projectId,"plans",origin.conversationId,undefined,origin.planVersion)}>Approved plan · version {origin.planVersion}</a></nav>
    {:else}<p class="text-sm text-muted-foreground">Direct submission · no originating plan</p>{/if}
    <section class="space-y-4" aria-labelledby="task-information-title">
      <div class="flex flex-wrap items-center justify-between gap-2"><h2 id="task-information-title" class="text-lg font-semibold">Information</h2>{#key task.id}<TaskUpdateStatus taskId={task.id} resourceVersion={task.resourceVersion} />{/key}</div>
      <InformationCard title="Task" status={task.state} actions={[
        ...(["failed", "cancelled", "deadline_exceeded"].includes(task.state) ? [{ label: "Retry task · new attempt", onSelect: () => retryForm?.requestSubmit(), disabled: busy }] : []),
        ...(terminal && canRetryPublication ? [{ label: "Retry publication", onSelect: () => publicationForm?.requestSubmit(), disabled: busy }] : []),
        ...(!terminal ? [{ label: "Cancel task", onSelect: () => cancelForm?.requestSubmit(), disabled: busy, destructive: true }] : []),
      ]}>
        <InformationTable rows={[
          { label: "Updated", value: formatDate(task.updatedAt) },
          { label: "Model", value: `${task.snapshot.agent.provider}/${task.snapshot.agent.model}` },
          ...(task.snapshot.providerBinding ? [{label:"LLM connection",value:task.snapshot.providerBinding.connectionId},{label:"Credential revision",value:task.snapshot.providerBinding.credentialRevision}] : []),
          { label: "Agent iterations", value: String(task.snapshot.limits.codingTurns) },
          { label: "Task ID", value: task.id, monospace: true },
          { label: "Accepted outcome", value: words(task.state) },
          { label: "Retry", value: ["failed", "cancelled", "deadline_exceeded"].includes(task.state) ? "New execution available" : canRetryPublication ? "Publication retry available" : "Unavailable" },
        ]} />
        {#if task.failure}<Feedback kind="error" title="Task failed" message={task.failure.message} />{/if}
        <form hidden bind:this={retryForm} method="POST" action="?/retry" use:enhance={submit}><input type="hidden" name="idempotencyKey" value={data.actionKeys.retry} /></form>
        <form hidden bind:this={publicationForm} method="POST" action="?/retryPublication" use:enhance={submit}><input type="hidden" name="idempotencyKey" value={data.actionKeys.retryPublication} /></form>
        <form hidden bind:this={cancelForm} method="POST" action="?/cancel" use:enhance={submit}><input type="hidden" name="idempotencyKey" value={data.actionKeys.cancel} /></form>
      </InformationCard>
    </section>

    <section class="space-y-4" aria-labelledby="task-delivery-title">
      <h2 id="task-delivery-title" class="text-lg font-semibold">Delivery</h2>
      {#each data.progress?.attempts ?? [] as attempt (attempt.id)}
        {@const receipt = attempt.pullRequest ?? attempt.result?.pullRequest}
        {@const publication = attempt.result?.publication ?? attempt.progress?.publication ?? "pending"}
          {#snippet pullRequest()}
            {#if receipt?.url}<a class="underline" href={receipt.url} target="_blank" rel="noreferrer">Open pull request #{receipt.number}</a>{:else}Waiting for worker delivery.{/if}
          {/snippet}
        <InformationCard title={`Pull request · attempt ${attempt.attemptNumber}`} status={publication}>
          <InformationTable rows={[
            { label: "Pull request", value: pullRequest },
            { label: "Quality checks", value: words(attempt.result?.quality?.outcome ?? "unavailable") },
            ...(receipt ? [{ label: "Head commit", value: receipt.headCommit, monospace: true }] : []),
          ]} />
          {#if receipt}<p class="text-sm text-muted-foreground">Published as a draft with the actual quality check result.</p>{/if}
        </InformationCard>
      {:else}<EmptyState title="Waiting for delivery" description="The worker publishes a draft pull request after completing the task and repository checks." />{/each}
    </section>

    <section class="space-y-4" aria-labelledby="task-progress-title">
      <h2 id="task-progress-title" class="text-lg font-semibold">Progress</h2>
      {#each data.progress?.attempts ?? [] as attempt (attempt.id)}
        <ExecutionOutcomes execution={attempt.result?.execution??attempt.state} quality={attempt.result?.quality?.outcome??attempt.progress?.latestFullCheck?.outcome??"unavailable"} publication={attempt.result?.publication??attempt.progress?.publication??"unavailable"} />
        <div id={`attempt-${attempt.id}`} class="scroll-mt-20"><InformationCard title={`Attempt ${attempt.attemptNumber}${attempt.publicationOnly ? " · publication retry" : ""}`} status={attempt.state}>
          <InformationTable rows={[
            { label: "Stage", value: words(attempt.progress?.phase ?? "unavailable") },
            { label: "Iterations used", value: `${attempt.result?.codingTurnsConsumed ?? attempt.progress?.codingTurnsConsumed ?? "Unavailable"} / ${attempt.snapshot.limits.codingTurns}` },
            { label: "Quality round", value: String(attempt.result?.qualityRound ?? attempt.progress?.qualityRound ?? "Unavailable") },
            { label: "Repair limit", value: String(attempt.snapshot.limits.qualityRepairLimit) },
            { label: "Reporting", value: words(attempt.reportingState) },
            { label: "Cleanup", value: words(attempt.cleanupState) },
            { label: "Input tokens", value: formatTokens(attempt.usage.input) },
            { label: "Output tokens", value: formatTokens(attempt.usage.output) },
            { label: "Model interactions", value: String(attempt.usage.interactions ?? "Unavailable") },
          ]} />
          {#if attempt.failure}<Feedback kind="error" title="Attempt needs attention" message={attempt.failure.message} />{/if}
          <WorkerActivity events={attempt.agentActivity ?? []} attemptNumber={attempt.attemptNumber} />
        </InformationCard></div>
      {:else}<EmptyState title="Waiting for the first attempt" description="Progress will appear when the worker starts." />{/each}
    </section>

    <section class="space-y-4" aria-labelledby="task-request-title">
      <h2 id="task-request-title" class="text-lg font-semibold">Task configuration</h2>
      <p class="text-sm text-muted-foreground">Each attempt keeps the repository, runtime, and request selected when the task was created.</p>
      <div class="grid gap-4 lg:grid-cols-2">
        <InformationCard title="Frozen request">
          <InformationTable rows={[
            { label: "Agent", value: task.snapshot.agent.agent },
            { label: "Model", value: `${task.snapshot.agent.provider}/${task.snapshot.agent.model}` },
          ...(task.snapshot.providerBinding ? [{label:"LLM connection",value:task.snapshot.providerBinding.connectionId},{label:"Credential revision",value:task.snapshot.providerBinding.credentialRevision}] : []),
            { label: "Agent iterations", value: String(task.snapshot.limits.codingTurns) },
            { label: "Deadline", value: task.snapshot.deadlineAt ? formatDate(task.snapshot.deadlineAt) : "None" },
            { label: "Test edits", value: task.snapshot.changeConstraints.allowTestEdits ? "Authorized" : "Protected" },
            { label: "Environment changes", value: task.snapshot.changeConstraints.environmentChange ? `Regenerate .ayni.lock for: ${task.snapshot.changeConstraints.environmentChange.dependencyPaths.join(", ")}` : "Protected" },
          ]} />
          <div class="space-y-2 text-sm"><h4 class="font-medium">Acceptance criteria</h4><ol class="list-decimal space-y-1 pl-5">{#each task.snapshot.acceptanceCriteria as criterion, index (index)}<li>{criterion}</li>{:else}<li class="list-none text-muted-foreground">None specified.</li>{/each}</ol></div>
          <div class="space-y-2 text-sm"><h4 class="font-medium">Protected paths</h4><ul class="space-y-1 font-mono text-xs [overflow-wrap:anywhere]">{#each task.snapshot.changeConstraints.protectedPaths as path, index (index)}<li>{path}</li>{:else}<li class="font-sans text-muted-foreground">None specified.</li>{/each}</ul></div>
        </InformationCard>
        <InformationCard title="Frozen repository and runtime">
          <InformationTable rows={[
            { label: "Repository revision", value: task.snapshot.repository.revision, monospace: true },
            { label: "Profile revision", value: task.snapshot.runtime.runtimeProfileRevisionId, monospace: true },
            { label: "OCI image", value: `${task.snapshot.runtime.ociRepository}@${task.snapshot.runtime.resolvedDigest}`, monospace: true },
            { label: "Platform", value: `${task.snapshot.runtime.platform.operatingSystem}/${task.snapshot.runtime.platform.architecture}` },
          ]} />
        </InformationCard>
      </div>
    </section>

    <section class="space-y-4" aria-labelledby="task-quality-title">
      <h2 id="task-quality-title" class="text-lg font-semibold">Quality</h2>
      <p class="text-sm text-muted-foreground">Full repository checks apply to the final committed source.</p>
      {#each data.progress?.attempts ?? [] as attempt (attempt.id)}
        {@const quality = attempt.result?.quality ?? attempt.progress?.latestFullCheck}
        <InformationCard title={`Quality · attempt ${attempt.attemptNumber}`} status={quality?.outcome ?? "unavailable"}>
          <InformationTable rows={[
            { label: "Completed targets", value: String(quality?.completedTargets ?? "Unavailable") },
            { label: "Skipped targets", value: String(quality?.skippedTargets ?? "Unavailable") },
            { label: "Warnings", value: String(quality?.warningOffenders ?? "Unavailable") },
            { label: "Failures", value: String(quality?.failingOffenders ?? "Unavailable") },
            ...(quality?.evidence ? [{ label: "Checked commit", value: quality.evidence.commit, monospace: true }, { label: "Evidence digest", value: quality.evidence.signalsDigest, monospace: true }] : []),
          ]} />
          <ul class="flex flex-wrap gap-2 text-sm" aria-label="Quality signals">{#each quality?.rows ?? [] as row, index (`${row.kind}:${index}`)}<li><QualitySignal {row} /></li>{/each}</ul>
          <ul class="space-y-1 text-sm text-destructive" aria-label="Actionable findings">{#each quality?.findings ?? [] as finding, index (index)}<li>{finding}</li>{/each}</ul>
        </InformationCard>
      {:else}<EmptyState title="Quality result unavailable" description="Repository check results will appear when the worker completes a full check." />{/each}
    </section>
  {/if}
</div>
