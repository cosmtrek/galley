<script lang="ts">
  import type { Block, Comment } from "../lib/types";

  let {
    blocks,
    comments,
    editable = true,
    onjump,
    onadd,
  }: {
    blocks: Block[];
    comments: Comment[];
    editable?: boolean;
    onjump: (id: string | null) => void;
    onadd: (sectionId: string | null) => void;
  } = $props();

  const headings = $derived(blocks.filter((b) => b.kind === "heading" && (b.level ?? 2) <= 3));

  const counts = $derived.by(() => {
    const m = new Map<string, number>();
    for (const c of comments) {
      if (c.status === "resolved") continue;
      const key = c.anchor.type === "document" ? "" : (c.section_id ?? "");
      m.set(key, (m.get(key) ?? 0) + 1);
    }
    return m;
  });
</script>

<aside class="outline" aria-label="大纲">
  <ul>
    <li class="doc"><div class="item" onclick={() => onjump(null)} onkeydown={(e) => e.key === "Enter" && onjump(null)} role="button" tabindex="0">
      <span class="t">整篇报告</span>
      {#if editable}
        <button class="add" title="整篇评论" onclick={(e) => { e.stopPropagation(); onadd(null); }}>＋</button>
        <span class="count" class:zero={!counts.get("")}>{counts.get("") ?? 0}</span>
      {/if}
    </div></li>
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
        {#if editable}
          <button class="add" title="评论本章节" onclick={(e) => { e.stopPropagation(); onadd(h.id); }}>＋</button>
          <span class="count" class:zero={!counts.get(h.id)}>{counts.get(h.id) ?? 0}</span>
        {/if}
      </div></li>
    {/each}
  </ul>
</aside>
