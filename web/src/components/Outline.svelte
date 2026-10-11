<script lang="ts">
  import type { Block, Comment } from "../lib/types";

  let {
    blocks,
    comments,
    showCounts = true,
    onjump,
  }: {
    blocks: Block[];
    comments: Comment[];
    showCounts?: boolean;
    onjump: (id: string) => void;
  } = $props();

  const headings = $derived(blocks.filter((b) => b.kind === "heading" && (b.level ?? 2) <= 3));

  const counts = $derived.by(() => {
    const m = new Map<string, number>();
    for (const c of comments) {
      if (c.status === "resolved" || c.anchor.type === "document" || !c.section_id) continue;
      m.set(c.section_id, (m.get(c.section_id) ?? 0) + 1);
    }
    return m;
  });
</script>

<aside class="outline" aria-label="目录">
  <div class="outline-head">目录</div>
  <ul>
    {#each headings as h (h.id)}
      <li class="l{h.level ?? 2}"><div
        class="item"
        onclick={() => onjump(h.id)}
        onkeydown={(e) => e.key === "Enter" && onjump(h.id)}
        role="button"
        tabindex="0"
        title={h.text}
      >
        <span class="t">{h.text}</span>
        {#if showCounts}
          <span class="count" class:zero={!counts.get(h.id)}>{counts.get(h.id) ?? 0}</span>
        {/if}
      </div></li>
    {/each}
  </ul>
</aside>
