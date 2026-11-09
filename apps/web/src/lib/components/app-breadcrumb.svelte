<script lang="ts">
  let { items }: { items: { href: string; label: string }[] } = $props();
</script>

<nav class="breadcrumb" aria-label="Breadcrumb">
  <ol role="list">
    {#each items as item, index (item.href)}
      <li>
        {#if index > 0}<span class="separator" aria-hidden="true">/</span>{/if}
        <a href={item.href} title={item.label} aria-current={index === items.length - 1 ? "page" : undefined}>
          {item.label}
        </a>
      </li>
    {/each}
  </ol>
</nav>

<style>
  .breadcrumb {
    min-width: 0;
    margin-right: auto;
    font-size: 12px;
    color: var(--muted-foreground);
  }
  ol, li { display: flex; align-items: center; gap: 12px; min-width: 0; }
  li:last-child { flex-shrink: 0; max-width: 60%; }
  li:only-child { max-width: 100%; }
  a { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  a:hover, a:focus-visible { color: var(--foreground); text-decoration: underline; }
  a[aria-current="page"] { color: var(--foreground); }
  .separator { opacity: .5; }
  @media (width <= 540px) {
    .breadcrumb { font-size: 11px; }
    ol, li { gap: 8px; }
  }
</style>
