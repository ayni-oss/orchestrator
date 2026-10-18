<script lang="ts">
  import type {ProjectViewData} from "$lib/project-view.js";
  import type {SubmitFunction} from "@sveltejs/kit";
  import {enhance} from "$lib/forms.js";
  import ProposalModelRevision from "$lib/components/proposal-model-revision.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import {Button} from "$lib/components/ui/button/index.js";
  import {words} from "$lib/presentation.js";
  let {data,proposal,allowDecision=true,modelRevision=false,planVersion=""}:{data:ProjectViewData;proposal:NonNullable<ProjectViewData["conversation"]>["proposals"][number];allowDecision?:boolean;modelRevision?:boolean;planVersion?:string}=$props();
  const conversation=$derived(data.conversation!);
  const active=$derived(data.project?.state==="active"&&data.workspace?.state==="active"&&!!data.agentConfiguration);
  let pending=$state(false);
  const submit:SubmitFunction=()=>{pending=true;return async({update})=>{try{await update({reset:false});}finally{pending=false;}};};
  function key(suffix:string){return `${data.idempotencyKey}-${suffix}`;}
  function name(id:string){return data.repositories.find(r=>r.id===id)?.displayName??id;}
  function actionTitle(action:string){return action==="create_task"?"Task plan":action==="cancel_task"?"Confirm cancellation":action==="retry_task"?"Confirm task retry":action==="retry_publication"?"Confirm publication retry":words(action);}
</script>
      <InformationCard title={actionTitle(proposal.action)} status={proposal.state}>
        {#if proposal.request && proposal.action==="create_task"}
          {@const request=proposal.request}
          <InformationTable rows={[
            {label:"Repository",value:name(request.repositoryConnectionId)},
            {label:"Source commit",value:request.repositoryRevision,monospace:true},
            {label:"Runtime",value:request.runtimeProfileId},
            ...(proposal.runtime ? [{label:"Runtime revision",value:proposal.runtime.profileRevisionId},{label:"Runtime image",value:`${proposal.runtime.image}@${proposal.runtime.digest}`}]:[]),
            {label:"Coding connection",value:data.llmConnections.find(c=>c.id===request.connectionId)?.displayName??request.connectionId??"Deployment default"},
            {label:"Provider / model",value:`${request.agent.provider} / ${request.agent.model}`},
            {label:"Model turns",value:String(request.codingTurns??24)},
            {label:"Deadline",value:`${request.deadlineSeconds??3600} seconds`},
            {label:"Protected paths",value:request.changeConstraints.protectedPaths.join(", ")||"Default protections"},
            {label:"Test edits",value:request.changeConstraints.allowTestEdits?"Allowed":"Protected"},
            {label:"Environment changes",value:request.changeConstraints.environmentChange?`Regenerate .ayni.lock for: ${request.changeConstraints.environmentChange.dependencyPaths.join(", ")}`:"Protected"},
          ]} />
          <div class="space-y-2"><h4 class="text-sm font-semibold">Implementation request</h4><p class="whitespace-pre-wrap break-words text-sm">{request.request}</p><h4 class="text-sm font-semibold">Acceptance criteria</h4><ul class="list-disc space-y-1 pl-5 text-sm">{#each request.acceptanceCriteria as criterion}<li>{criterion}</li>{/each}</ul></div>
        {:else}<p class="text-sm">{proposal.action==="cancel_task"?"Cancel the running task and retain its history.":proposal.action==="retry_publication"?"Retry publishing the retained result; coding will not run again.":"Start a new coding attempt using this task’s frozen inputs."} <a class="underline" href={`/_old/tasks/${encodeURIComponent(proposal.targetTaskId)}`}>Open target task</a></p>{/if}
        {#if proposal.failure}<Feedback kind="warning" title="Action needs attention" message={proposal.failure} />{/if}
        {#if proposal.state==="pending" && active && allowDecision}
          <form method="POST" action="?/decideProposal" use:enhance={submit} class="flex flex-wrap gap-2"><input type="hidden" name="idempotencyKey" value={key(`decision-${proposal.id}-${conversation.updateCursor}`)} /><input type="hidden" name="conversationId" value={conversation.id} /><input type="hidden" name="proposalId" value={proposal.id} /><Button type="submit" name="decision" value="approve" disabled={pending}>{proposal.action==="create_task"?"Approve and submit task":"Confirm action"}</Button><Button type="submit" name="decision" value="revise" variant="outline" disabled={pending}>Request revision</Button><Button type="submit" name="decision" value="reject" variant="outline" disabled={pending}>Reject</Button></form>
          <p class="text-xs text-muted-foreground">Approval submits these exact requirements. For a different coding model, connection, or scope, request a revision and send the changes in a message.</p>
        {/if}
        {#if proposal.taskId}
          {@const task=data.tasks.find(t=>t.id===proposal.taskId)}
          {@const attempt=data.taskProgress[proposal.taskId]?.attempts.at(-1)}
          {@const receipt=attempt?.pullRequest??attempt?.result?.pullRequest}
          <div class="space-y-2 border-t pt-3"><a class="font-medium underline" href={`/_old/tasks/${encodeURIComponent(proposal.taskId)}`}>View task {task?`· ${words(task.state)}`:""}</a>
            {#if attempt}<InformationTable rows={[{label:"Execution",value:attempt.result?.execution??attempt.progress?.phase??words(attempt.state)},{label:"Quality",value:attempt.result?.quality?.outcome??"Pending"},{label:"Publication",value:attempt.result?.publication??attempt.progress?.publication??"Pending"}]} />{#if attempt.requestedInput}<p class="whitespace-pre-wrap text-sm">Input requested: {attempt.requestedInput}</p>{/if}{#if receipt?.url}<Button href={receipt.url} variant="outline" target="_blank" rel="noopener noreferrer">Open draft PR #{receipt.number}</Button>{/if}{/if}
          </div>
        {/if}
{#if modelRevision && active && proposal.request}<ProposalModelRevision {proposal} connections={data.llmConnections} catalogues={data.connectionModels} conversationId={conversation.id} planId={conversation.plan?.id??""} {planVersion} idempotencyKey={key(`model-${proposal.id}-${conversation.updateCursor}`)} />{/if}
</InformationCard>
