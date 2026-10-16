<script lang="ts">
  import type { AgentProfileSettingsDto } from "$lib/project-view.js";

  let { profile, detailed = false }: { profile: AgentProfileSettingsDto; detailed?: boolean } = $props();
  const duration = $derived.by(() => {
    const seconds = Number(profile.deadlineSeconds);
    const parts = [
      { value: Math.floor(seconds / 3600), unit: "hour" },
      { value: Math.floor(seconds % 3600 / 60), unit: "minute" },
      { value: seconds % 60, unit: "second" },
    ].filter(part => part.value > 0);
    return parts.map(({ value, unit }) => `${value} ${unit}${value === 1 ? "" : "s"}`).join(" ") || "0 seconds";
  });
</script>

<div class="min-w-0 space-y-4 rounded-lg bg-muted/40 p-4 text-sm">
  <dl class="grid gap-4 sm:grid-cols-2">
    <div class="space-y-1">
      <dt class="text-xs font-medium text-muted-foreground">Model call limit</dt>
      <dd class="font-medium">Up to {profile.maxModelCalls} model calls</dd>
    </div>
    <div class="space-y-1">
      <dt class="text-xs font-medium text-muted-foreground">Time limit</dt>
      <dd class="font-medium">{duration}</dd>
    </div>
    <div class="min-w-0 space-y-2 sm:col-span-2">
      <dt class="text-xs font-medium text-muted-foreground">Allowed tools</dt>
      <dd>
        <ul class="flex flex-wrap gap-2" aria-label="Allowed tools">
          {#each profile.allowedTools as tool (tool)}
            <li class="max-w-full rounded-md border bg-background px-2 py-1"><code class="break-all text-xs">{tool}</code></li>
          {:else}<li class="text-muted-foreground">No tools allowed.</li>{/each}
        </ul>
      </dd>
    </div>
    {#if detailed}
      <div class="min-w-0 space-y-2 sm:col-span-2">
        <dt class="text-xs font-medium text-muted-foreground">Instructions</dt>
        <dd class="whitespace-pre-wrap break-words rounded-md border bg-background p-3 leading-relaxed">{profile.instructions || "Application role instructions"}</dd>
      </div>
      <div class="min-w-0 space-y-2 sm:col-span-2">
        <dt class="text-xs font-medium text-muted-foreground">Repository scope</dt>
        <dd>
          {#if profile.repositoryIds.length}
            <ul class="space-y-1" aria-label="Repository scope">{#each profile.repositoryIds as id (id)}<li class="break-all font-mono text-xs">{id}</li>{/each}</ul>
          {:else}Current repository membership{/if}
        </dd>
      </div>
      <div class="min-w-0 space-y-2 sm:col-span-2">
        <dt class="text-xs font-medium text-muted-foreground">Pinned skills</dt>
        <dd>
          {#if profile.skills.length}
            <ul class="space-y-3" aria-label="Pinned skills">
              {#each profile.skills as skill (skill.id)}
                <li class="space-y-1 rounded-md border bg-background p-3">
                  <p class="break-words font-medium">{skill.id} <span class="break-all font-mono text-xs text-muted-foreground">@ {skill.revision}</span></p>
                  <p class="whitespace-pre-wrap break-words leading-relaxed">{skill.instructions}</p>
                </li>
              {/each}
            </ul>
          {:else}<span class="text-muted-foreground">No pinned skills.</span>{/if}
        </dd>
      </div>
    {/if}
  </dl>
</div>
