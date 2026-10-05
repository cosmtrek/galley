<script lang="ts">
  import type { ReportInfo, Round } from "../lib/types";
  import CopyButton from "./CopyButton.svelte";

  let { report, round }: { report: ReportInfo; round: Round } = $props();

  const prompt = $derived(
    `处理 Galley 报告「${report.title}」第 ${round.seq} 轮评论（report_id: ${report.id}）：用 galley MCP 的 galley_get_round 读取评论，按评论修改后用 galley_submit_round 一次性提交新 Markdown、逐条回复和本轮摘要。`,
  );
  let failed = $state(false);
</script>

<div class="row">
  <span class="muted small">Galley 不会自己调用 AI，把指令发给已接入的 AI 工具。<a href="/app#agent">还没接入？</a></span>
  <span class="spacer"></span>
  <CopyButton text={prompt} label="复制指令" small bind:failed />
</div>
{#if failed}<code class="prompt">{prompt}</code>{/if}
