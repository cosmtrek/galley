<script lang="ts">
  // Placeholder lines for text that is still loading; put it inside a .skeleton region.
  let { lines = 3 }: { lines?: number } = $props();
</script>

<div class="sk-lines" aria-hidden="true">
  {#each { length: lines }, i (i)}<span class="sk"></span>{/each}
</div>
