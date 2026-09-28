<script lang="ts">
  import { Check, Circle, LoaderCircle, X } from "@lucide/svelte";
  import { stageLabel, words } from "$lib/presentation.js";

  type Stage = "queue" | "coding" | "validation" | "pr_preparation";
  type Outcome = "queued" | "started" | "succeeded" | "blocked" | "failed" | "cancelled" | "unsupported" | "unknown";
  interface TaskStagesView {
    phase: string;
    lifecycle: Array<{ stage: string; outcome: string }>;
  }

  let { task }: { task: TaskStagesView } = $props();
  const stages: Stage[] = ["queue", "coding", "validation", "pr_preparation"];
  const activeStage = $derived(task.phase === "queued" ? "queue" : task.phase === "coding" || task.phase === "waiting_for_input" ? "coding" : task.phase === "validation_queued" || task.phase === "validating" ? "validation" : task.phase === "pr_queued" || task.phase === "preparing_pr" ? "pr_preparation" : undefined);
  const latestOutcome = (stage: Stage): Outcome | undefined => task.lifecycle.filter((event) => event.stage === stage).at(-1)?.outcome as Outcome | undefined;
  const successful = (outcome: Outcome | undefined): boolean => outcome === "succeeded";
  const failed = (outcome: Outcome | undefined): boolean => ["failed", "blocked", "cancelled", "unsupported"].includes(outcome ?? "");
</script>

<ol class="grid gap-2 sm:grid-cols-4" aria-label="Task stages">
  {#each stages as stage, index}
    {@const outcome = latestOutcome(stage)}
    <li class="relative rounded-lg border p-3" class:border-primary={activeStage === stage} class:bg-accent={activeStage === stage}>
      <div class="flex items-center gap-2">
        <span class="grid size-6 shrink-0 place-items-center rounded-full border text-muted-foreground" aria-hidden="true">
          {#if successful(outcome)}<Check class="size-3.5 text-emerald-600" />{:else if failed(outcome)}<X class="size-3.5 text-red-600" />{:else if activeStage === stage}<LoaderCircle class="size-3.5 motion-safe:animate-spin" />{:else}<Circle class="size-2" />{/if}
        </span>
        <div><p class="text-sm font-medium">{stageLabel(stage)}</p><p class="text-xs text-muted-foreground">{outcome ? words(outcome) : index === 0 ? "Pending" : "Not started"}</p></div>
      </div>
    </li>
  {/each}
</ol>
