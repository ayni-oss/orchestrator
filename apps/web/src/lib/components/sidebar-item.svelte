<script lang="ts">
  import type { Snippet } from 'svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  let { href, active=false, disabled=false, onclick, expanded, controls, children }: {
    href?:string;active?:boolean;disabled?:boolean;onclick?:()=>void;
    expanded?:boolean;controls?:string;children:Snippet;
  }=$props();
</script>
<Button variant="ghost" class="h-10 w-full justify-start gap-3 px-3 text-sm aria-disabled:pointer-events-none aria-disabled:opacity-50 data-[active=true]:bg-muted data-[active=true]:text-foreground" {href} {disabled} {onclick} data-active={active} aria-current={active&&href?'page':undefined} aria-expanded={expanded} aria-controls={controls}>
  {@render children()}
</Button>
