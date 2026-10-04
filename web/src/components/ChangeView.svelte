<script lang="ts">
  import { BLOCK_KIND_LABEL, CHANGE_LABEL } from "../lib/format";
  import { partner, RICH_KINDS, type DiffCtx } from "../lib/diffctx";
  import type { BlockChange } from "../lib/types";

  let { change, ctx = null, showOp = true }: { change: BlockChange; ctx?: DiffCtx | null; showOp?: boolean } = $props();

  // A replacement is shown from the deleted block's point of view whichever half we were handed.
  const pair = $derived(partner(ctx, change));
  const before = $derived(change.op === "added" ? pair : change);
  const after = $derived(change.op === "deleted" ? pair : change);
  const oldBlock = $derived(before && before.op !== "added" ? ctx?.old.get(before.block_id) : undefined);
  const newBlock = $derived(after && after.op !== "deleted" ? ctx?.new.get(after.block_id) : undefined);
  const kindChanged = $derived(!!oldBlock && !!newBlock && oldBlock.kind !== newBlock.kind);
  const unchangedMove = $derived(change.op === "moved" && !change.segments);
  const rich = $derived(
    !unchangedMove &&
      (kindChanged || (!!oldBlock && RICH_KINDS.has(oldBlock.kind)) || (!!newBlock && RICH_KINDS.has(newBlock.kind))),
  );
  const shape = (cells?: string[][]) => (cells ? `${cells.length}x${Math.max(0, ...cells.map((r) => r.length))}` : "");
  // A cell list reads well for a few edits; added rows/columns or wholesale rewrites are clearer rendered.
  const tableEdit = $derived(
    change.op !== "added" &&
      change.op !== "deleted" &&
      change.kind === "table" &&
      !kindChanged &&
      !!change.cells?.length &&
      (!oldBlock || !newBlock || (shape(oldBlock.cells) === shape(newBlock.cells) && change.cells.length <= 6)),
  );
  const label = $derived(
    pair
      ? kindChanged && oldBlock && newBlock
        ? `替换：${BLOCK_KIND_LABEL[oldBlock.kind]} → ${BLOCK_KIND_LABEL[newBlock.kind]}`
        : "替换"
      : CHANGE_LABEL[change.op],
  );
</script>

{#if showOp}<div class="change-op" class:warn={kindChanged}>{label}</div>{/if}
{#if tableEdit}
  <table class="cells">
    <thead><tr><th>单元格</th><th>修改前</th><th>修改后</th></tr></thead>
    <tbody>
      {#each change.cells ?? [] as c (c.row + "," + c.col)}
        <tr>
          <td class="muted">第 {c.row + 1} 行第 {c.col + 1} 列</td>
          <td class="diff"><del>{c.old ?? ""}</del></td>
          <td class="diff"><ins>{c.new ?? ""}</ins></td>
        </tr>
      {/each}
    </tbody>
  </table>
{:else if rich && (oldBlock || newBlock)}
  {#if oldBlock && newBlock}
    <div class="side">
      <div><div class="side-label">修改前</div><div class="report rendered old">{@html oldBlock.html}</div></div>
      <div><div class="side-label">修改后</div><div class="report rendered new">{@html newBlock.html}</div></div>
    </div>
  {:else if newBlock}
    <div class="report rendered new">{@html newBlock.html}</div>
  {:else if oldBlock}
    <div class="report rendered old gone">{@html oldBlock.html}</div>
  {/if}
{:else if pair && before && after}
  <div class="diff deleted">{before.old_text}</div>
  <div class="diff added"><ins>{after.new_text}</ins></div>
{:else if change.op === "added"}
  <div class="diff added"><ins>{change.new_text}</ins></div>
{:else if change.op === "deleted"}
  <div class="diff deleted">{change.old_text}</div>
{:else if change.kind === "diagram"}
  <div class="muted small">{change.op === "moved" ? "图表位置移动" : "图表源码已修改"}。</div>
{:else if change.segments}
  <div class="diff">
    {#each change.segments as s, i (i)}{#if s.op === "ins"}<ins>{s.text}</ins>{:else if s.op === "del"}<del>{s.text}</del>{:else}{s.text}{/if}{/each}
  </div>
{:else}
  <div class="diff">{change.new_text}</div>
  {#if change.op === "moved"}<div class="muted small">位置移动，文字未改。</div>{/if}
{/if}
