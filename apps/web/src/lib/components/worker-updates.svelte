<script lang="ts">
  import type {ProjectViewData} from "$lib/project-view.js";
  type ProjectWorkerUpdateDto=NonNullable<ProjectViewData["conversation"]>["workerUpdates"][number];
  import ExecutionOutcomes from "$lib/components/execution-outcomes.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import {Button} from "$lib/components/ui/button/index.js";
  import {projectHref} from "$lib/project-navigation.js";
  import {words,formatDate} from "$lib/presentation.js";
  let {updates}:{updates:ProjectWorkerUpdateDto[]}=$props();
</script>
{#if updates.length}
  <section class="space-y-4" aria-label="Worker updates">
    <h3 class="font-semibold">Worker updates</h3>
    <p class="text-xs text-muted-foreground">Accepted progress and results, with one card per attempt. Older evidence remains in saved history and task details. A published draft does not imply passing quality or approval to merge.</p>
    {#each updates as update (update.attemptId)}
      {@const quality=update.result?.quality??update.progress?.latestFullCheck}
      {@const receipt=update.pullRequest??update.result?.pullRequest}
      {@const question=update.requestedInput??update.result?.requestedInput}
      <InformationCard title={`Task attempt ${update.attemptNumber}${update.publicationOnly?" · publication retry":""}`} status={update.state}>
        <p class="break-all text-xs text-muted-foreground">Task {update.taskId} · Attempt {update.attemptId}{update.updatedAt?` · ${formatDate(update.updatedAt)}`:""}</p>
        <ExecutionOutcomes progress={update.progress?.phase??update.state} execution={update.result?.execution??update.state} quality={quality?.outcome??"unavailable"} publication={update.publicationOnly&&["queued","running"].includes(update.state)?"Retry in progress":update.result?.publication??update.progress?.publication??"unavailable"} />
        {#if update.failure}<Feedback kind="warning" title="Execution needs attention" message={update.failure} />{/if}
        {#if question}
          <Feedback kind="warning" title="Worker question" message={question} />
          <p class="text-xs text-muted-foreground">Reply in this conversation to prepare a follow-up plan for approval. This does not resume the waiting attempt. Open task controls to cancel it or inspect retry options.</p>
          <Button variant="outline" href={`${projectHref(update.origin.projectId,"overview",update.origin.conversationId)}&question=${encodeURIComponent(update.attemptId)}`}>Discuss this question</Button>
        {/if}
        {#if quality?.findings.length}<div class="space-y-2 text-sm"><h4 class="font-medium">Unresolved findings</h4><ul class="list-disc space-y-1 pl-5">{#each quality.findings as finding,index (index)}<li class="break-words">{finding}</li>{/each}</ul></div>{/if}
        {#if update.result?.commit}<p class="break-all font-mono text-xs">Commit: {update.result.commit}</p>{/if}
        {#if update.result?.changedPaths.length}<details class="text-sm"><summary class="cursor-pointer">Changed paths · {update.result.changedPaths.length}</summary><ul class="mt-2 space-y-1 break-all font-mono text-xs">{#each update.result.changedPaths as path,index (index)}<li>{path}</li>{/each}</ul></details>{/if}
        {#if quality?.evidence}<details class="text-sm"><summary class="cursor-pointer">Quality evidence</summary><InformationTable rows={[{label:"Checked commit",value:quality.evidence.commit},{label:"Signals digest",value:quality.evidence.signalsDigest},{label:"Source fingerprint",value:quality.evidence.sourceFingerprint}]} /></details>{:else if update.result}<p class="text-sm text-muted-foreground">Quality evidence unavailable. A delivery receipt cannot replace missing evidence.</p>{/if}
        <div class="flex flex-wrap gap-2">
          <Button variant="outline" href={`/tasks/${encodeURIComponent(update.taskId)}#${encodeURIComponent(`attempt-${update.attemptId}`)}`}>Open execution attempt</Button>
          <Button variant="outline" href={projectHref(update.origin.projectId,"plans",update.origin.conversationId,undefined,update.origin.planVersion)}>Approved plan · version {update.origin.planVersion}</Button>
          {#if receipt?.url}<Button variant="outline" href={receipt.url} target="_blank" rel="noopener noreferrer">{receipt.draft?"Open draft PR":"Open pull request"} #{receipt.number}</Button>{/if}
        </div>
      </InformationCard>
    {/each}
  </section>
{/if}
