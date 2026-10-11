<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    description,
    actions,
  }: { title: string; description?: string | Snippet; actions?: Snippet } = $props();
</script>

<header class="page-head">
  <div>
    <h1>{title}</h1>
    {#if typeof description === "string"}
      <p class="muted">{description}</p>
    {:else if description}
      <p class="muted">{@render description()}</p>
    {/if}
  </div>
  {#if actions}<div class="page-head-actions">{@render actions()}</div>{/if}
</header>
