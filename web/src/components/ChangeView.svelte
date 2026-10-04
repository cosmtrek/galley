<script lang="ts">
  import { CHANGE_LABEL } from "../lib/format";
  import type { BlockChange } from "../lib/types";

  let { change, showOp = true }: { change: BlockChange; showOp?: boolean } = $props();
  const tableEdit = $derived(change.kind === "table" && change.cells && change.cells.length > 0);
</script>

{#if showOp}<div class="change-op">{CHANGE_LABEL[change.op]}</div>{/if}
{#if change.op === "added"}
  <div class="diff added"><ins>{change.new_text}</ins></div>
{:else if change.op === "deleted"}
  <div class="diff deleted">{change.old_text}</div>
{:else if tableEdit}
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
{:else if change.segments}
  <div class="diff">
    {#each change.segments as s, i (i)}{#if s.op === "ins"}<ins>{s.text}</ins>{:else if s.op === "del"}<del>{s.text}</del>{:else}{s.text}{/if}{/each}
  </div>
{:else}
  <div class="diff">{change.new_text}</div>
  {#if change.op === "moved"}<div class="muted small">位置移动，文字未改。</div>{/if}
{/if}
