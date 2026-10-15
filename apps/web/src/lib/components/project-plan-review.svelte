<script lang="ts">
  import {untrack} from "svelte";
  import type {PageData} from "../../routes/projects/[projectId]/$types";
  import type {SubmitFunction} from "@sveltejs/kit";
  import {enhance} from "$lib/forms.js";
  import InformationCard from "$lib/components/information-card.svelte";
  import ProjectProposal from "$lib/components/project-proposal.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import {Button} from "$lib/components/ui/button/index.js";
  import {Label} from "$lib/components/ui/label/index.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import {formatDate,words} from "$lib/presentation.js";
  let {data}:{data:PageData}=$props();
  const conversation=$derived(data.conversation!);
  const plan=$derived(conversation.plan!);
  let selected=$state(untrack(()=>data.conversation?.plan?.versions.at(-1)?.version??""));
  const latest=$derived(plan.versions.at(-1));
  const version=$derived(plan.versions.find(v=>v.version===selected));
  const current=$derived(version?.version===latest?.version);
  const reviewable=$derived(current&&version?.state==="pending"&&data.project?.state==="active"&&data.workspace?.state==="active");
  let pending=$state(false);
  const submit:SubmitFunction=()=>{pending=true;return async({update})=>{try{await update({reset:false});}finally{pending=false;}};};
</script>
{#if version}
  <InformationCard title="Plan review" status={version.state}>
    <div class="space-y-2"><Label for="plan-version">Plan version</Label><ThemedSelect id="plan-version" value={version.version} onValueChange={value=>selected=value} options={plan.versions.map(v=>({value:v.version,label:`Version ${v.version} · ${words(v.state)}`}))} /></div>
    <p class="text-xs text-muted-foreground">Version {version.version} · {formatDate(version.createdAt)} · Plan {plan.id}</p>
    {#if !current}<p role="status" class="text-sm text-muted-foreground">Historical version · read only. A newer plan version is available; select it to review before deciding.</p>{/if}
    <div class="space-y-2"><h3 class="font-semibold">Findings</h3><p class="whitespace-pre-wrap break-words text-sm">{version.findings||"Unavailable — findings were not recorded for this plan."}</p></div>
    <div class="space-y-2"><h3 class="font-semibold">Scope</h3><p class="whitespace-pre-wrap break-words text-sm">{version.scope||"Unavailable — scope was not recorded for this plan."}</p></div>
    <div class="space-y-2"><h3 class="font-semibold">Acceptance criteria</h3><ul class="list-disc space-y-1 pl-5 text-sm">{#each version.acceptanceCriteria as criterion}<li>{criterion}</li>{:else}<li>Unavailable — review each task’s criteria below.</li>{/each}</ul></div>
    <div class="space-y-2"><h3 class="font-semibold">Dependency and environment impact</h3><p class="whitespace-pre-wrap break-words text-sm">{version.dependencyEnvironmentImpact||"Unavailable — dependency and environment impact has not been assessed."}</p><p class="text-xs text-muted-foreground">This assessment describes expected changes. Plan approval does not authorize environment or dependency mutations.</p></div>
    {#if version.failure}<Feedback kind="warning" title="Plan needs attention" message={version.failure} />{/if}
    <div class="space-y-4" aria-label="Proposed tasks">
      {#each conversation.proposals.filter(p=>version.proposalIds.includes(p.id)) as proposal (proposal.id)}<ProjectProposal {data} {proposal} allowDecision={false} modelRevision={reviewable} planVersion={version.version} />{/each}
    </div>
    {#if reviewable}
      <form method="POST" action="?/decidePlan" use:enhance={submit} class="flex flex-wrap gap-2">
        <input type="hidden" name="idempotencyKey" value={`${data.idempotencyKey}-plan-${plan.id}-${version.version}`} /><input type="hidden" name="conversationId" value={conversation.id} /><input type="hidden" name="planId" value={plan.id} /><input type="hidden" name="version" value={version.version} />
        <Button type="submit" name="decision" value="approve" disabled={pending}>Approve version {version.version} and submit tasks</Button><Button type="submit" name="decision" value="revise" variant="outline" disabled={pending}>Request revision</Button><Button type="submit" name="decision" value="reject" variant="outline" disabled={pending}>Reject version {version.version}</Button>
      </form>
      <p class="text-xs text-muted-foreground">Approval submits this exact version and its frozen task inputs. If source or runtime inputs have changed, request a revised plan and review it again.</p>
    {/if}
  </InformationCard>
{/if}
