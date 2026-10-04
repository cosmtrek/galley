<script lang="ts">
  import { onMount, tick } from "svelte";
  import { get, post } from "../lib/api";
  import { fmtTime, ROUND_LABEL } from "../lib/format";
  import { navigate } from "../lib/router.svelte";
  import type { ReportInfo } from "../lib/types";
  import TopBar from "../components/TopBar.svelte";

  let reports = $state<ReportInfo[] | null>(null);
  let error = $state("");
  let importing = $state(false);
  let markdown = $state("");
  let publicUrl = $state(location.origin);

  async function load() {
    try {
      reports = await get<ReportInfo[]>("/api/reports");
      const meta = await get<{ public_url: string }>("/api/meta");
      publicUrl = meta.public_url || location.origin;
    } catch (e) {
      error = String(e);
    }
    if (location.hash === "#agent") {
      await tick();
      document.getElementById("agent")?.scrollIntoView({ block: "start" });
    }
  }

  async function create() {
    try {
      const r = await post<ReportInfo>("/api/reports", { markdown });
      navigate(`/app/r/${r.id}`);
    } catch (e) {
      error = (e as Error).message;
    }
  }

  async function logout() {
    await post("/api/logout");
    location.reload();
  }

  const unresolved = (r: ReportInfo) => r.counts.draft + r.counts.open + r.counts.clarify + r.counts.verify + r.counts.orphaned;

  onMount(load);
</script>

<TopBar>
  <button class="quiet" onclick={() => (importing = !importing)}>导入 Markdown</button>
  <button class="quiet" onclick={logout}>退出</button>
</TopBar>

<div class="page">
  <h1>报告</h1>
  {#if error}<p class="error">{error}</p>{/if}

  {#if importing}
    <div class="panel">
      <h3>导入 Markdown</h3>
      <p class="muted small">通常由 AI 推送；这里可以手动粘贴一份报告作为 v1。</p>
      <textarea rows="10" bind:value={markdown} placeholder={"---\ntitle: 报告标题\nsummary: 一句话摘要\n---\n\n## 一、第一章\n\n正文……"}></textarea>
      <div class="row" style="margin-top: 8px">
        <button class="primary" disabled={!markdown.trim()} onclick={create}>创建</button>
        <button onclick={() => (importing = false)}>取消</button>
      </div>
    </div>
  {/if}

  {#if reports === null}
    <p class="muted">加载中…</p>
  {:else if reports.length === 0}
    <p class="muted">还没有报告。让 AI 通过下方的 MCP 或 API 推送一份，或点右上角「导入 Markdown」。</p>
  {:else}
    <table class="list">
      <thead>
        <tr><th>标题</th><th>版本</th><th>轮次</th><th>未解决</th><th>发布</th><th>更新</th></tr>
      </thead>
      <tbody>
        {#each reports as r (r.id)}
          <tr>
            <td>
              <a href="/app/r/{r.id}">{r.title}</a>
              {#if r.summary}<div class="muted small">{r.summary}</div>{/if}
            </td>
            <td>v{r.current_seq}</td>
            <td>
              {#if r.active_round}
                <span class="badge" class:s-verify={r.active_round.status === "verifying"}>
                  第 {r.active_round.seq} 轮 · {ROUND_LABEL[r.active_round.status]}
                </span>
              {:else}<span class="muted">-</span>{/if}
            </td>
            <td>{unresolved(r) || "-"}</td>
            <td>{#if r.publication}v{r.publication.version_seq}{:else}<span class="muted">-</span>{/if}</td>
            <td class="muted">{fmtTime(r.updated_at)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}

  <h2 id="agent">接入 AI</h2>
  <div class="panel">
    <h3>MCP（Claude Code 示例）</h3>
    <pre class="cmd">claude mcp add --transport http galley {publicUrl}/mcp --header "Authorization: Bearer &lt;agent_token&gt;"</pre>
    <p class="muted small">
      agent_token 见服务端 <code>data/secrets.json</code> 或环境变量 <code>GALLEY_AGENT_TOKEN</code>。之后在 agent 里说「处理 galley 评论」即可。
    </p>
    <h3 style="margin-top: 16px">HTTP API</h3>
    <pre class="cmd">curl -H "Authorization: Bearer $GALLEY_AGENT_TOKEN" {publicUrl}/api/pending
curl -H "Authorization: Bearer $GALLEY_AGENT_TOKEN" "{publicUrl}/api/reports/&lt;id&gt;/rounds/current?format=md"</pre>
  </div>
</div>
