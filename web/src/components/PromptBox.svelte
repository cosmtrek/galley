<script lang="ts">
  import { copyText } from "../lib/clipboard";
  import type { ReportInfo, Round } from "../lib/types";

  let { report, round }: { report: ReportInfo; round: Round } = $props();

  const prompt = $derived(
    `处理 Galley 报告「${report.title}」第 ${round.seq} 轮评论（report_id: ${report.id}）：用 galley MCP 的 galley_get_round 读取评论，按评论修改后用 galley_submit_round 一次性提交新 Markdown、逐条回复和本轮摘要。`,
  );
  let copied = $state<"" | "ok" | "fail">("");

  async function copy() {
    copied = (await copyText(prompt)) ? "ok" : "fail";
    setTimeout(() => (copied = ""), 2000);
  }
</script>

<div class="muted small">Galley 不会自己调用 AI。把这句话发给已接入 Galley 的 AI 工具：</div>
<code class="prompt">{prompt}</code>
<div class="row">
  <a class="small" href="/app#agent">还没接入 AI？</a>
  <span class="spacer"></span>
  {#if copied === "fail"}<span class="small muted">无法自动复制，请手动选中上面的文字</span>{/if}
  <button class="primary" onclick={copy}>{copied === "ok" ? "已复制" : "复制"}</button>
</div>
