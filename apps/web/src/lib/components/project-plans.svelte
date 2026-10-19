<script lang="ts">
  import type {ProjectViewData} from "$lib/project-view.js";
  import {invalidate} from "$app/navigation";
  import ProjectPlanReview from "$lib/components/project-plan-review.svelte";
  import ProjectProposal from "$lib/components/project-proposal.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import {Button} from "$lib/components/ui/button/index.js";
  import {projectHref} from "$lib/project-navigation.js";
  let {data,href=projectHref}:{data:ProjectViewData;href?:typeof projectHref}=$props();
</script>
<section class="space-y-4" aria-label="Project plans">
  <h2 class="text-lg font-semibold">Plans</h2>
  {#if data.conversationError}<Feedback kind="warning" title="Conversations unavailable" message={data.conversationError.message} /><Button variant="outline" onclick={()=>invalidate("app:conversations")}>Retry conversations</Button>{/if}
  {#if data.conversation}
    <p class="text-sm text-muted-foreground">{data.conversation.title}</p>
    {#if data.conversation.plan}{#key `${data.conversation.plan.id}-${data.planVersion}`}<ProjectPlanReview {data} {href} />{/key}
    {:else}
      <p class="text-sm text-muted-foreground">No plan yet. Discuss the project goal in <a class="underline" href={href(data.project!.id,"overview",data.conversation.id)}>Conversation</a> to prepare a plan for review.</p>
      {#each data.conversation.proposals.filter(proposal=>proposal.action==="create_task") as proposal (proposal.id)}<ProjectProposal {data} {proposal} allowDecision={false} />{/each}
    {/if}
  {:else if !data.conversationError}
    <p class="text-sm text-muted-foreground">Choose a conversation to see its plan, or start a chat to prepare one.</p>
  {/if}
</section>
