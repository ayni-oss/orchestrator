<script lang="ts">
  import * as Card from '$lib/components/ui/card/index.js';

  let { name, logo, description, href }: {
    name: string;
    logo: string;
    description: string;
    href: string;
  } = $props();
</script>

<a {href} aria-label={name} class="group block rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background">
  <Card.Root class="h-full transition-colors group-hover:bg-accent/30">
    <Card.Header>
      <Card.Title>
        <h2 class="flex items-center gap-3">
          <img src={`/provider-logos/${logo}.svg`} alt="" class="size-6 shrink-0 invert dark:invert-0" />
          {name}
        </h2>
      </Card.Title>
    </Card.Header>
    <Card.Content><p class="text-sm text-muted-foreground">{description}</p></Card.Content>
  </Card.Root>
</a>
