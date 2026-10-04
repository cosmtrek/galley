<script lang="ts">
  import { copyText } from "../lib/clipboard";

  let { publicUrl, token }: { publicUrl: string; token: string } = $props();

  let showToken = $state(false);
  let copied = $state<"" | "ok" | "fail">("");
  let client = $state("claude");
  const masked = $derived(token ? `${token.slice(0, 4)}${"•".repeat(12)}` : "<agent_token>");
  const mcp = $derived(`${publicUrl.replace(/\/$/, "")}/mcp`);
  const clients = $derived([
    {
      key: "claude",
      name: "Claude Code",
      cmd: (t: string) => `claude mcp add --transport http galley ${mcp} --header "Authorization: Bearer ${t}"`,
    },
    {
      key: "codex",
      name: "Codex",
      // Codex reads the token from the environment when it connects, so the export must persist (e.g. in ~/.zshrc).
      cmd: (t: string) => `export GALLEY_AGENT_TOKEN=${t}\ncodex mcp add galley --url ${mcp} --bearer-token-env-var GALLEY_AGENT_TOKEN`,
      note: "export 要写进 shell 配置（如 ~/.zshrc），Codex 连接时才读得到。",
    },
    {
      key: "droid",
      name: "Droid",
      cmd: (t: string) => `droid mcp add galley ${mcp} --type http --no-oauth --header "Authorization: Bearer ${t}"`,
    },
    {
      key: "devin",
      name: "Devin",
      cmd: (t: string) => `devin mcp add -s user galley ${mcp} -H "Authorization: Bearer ${t}"`,
      note: "-s user 写入全局配置，在任何目录都能用；去掉则只对当前项目生效。",
    },
  ]);
  const current = $derived(clients.find((c) => c.key === client) ?? clients[0]);

  async function copyCmd() {
    copied = (await copyText(current.cmd(token))) ? "ok" : "fail";
    setTimeout(() => (copied = ""), 2000);
  }
</script>

<p class="small" style="margin: 0 0 8px">
  在终端运行一次下面的命令，把 Galley 接到你的 AI 工具。之后每次在 Galley 提交一轮评论，到 AI 工具里说「处理 Galley 评论」即可；Galley 不会主动调用 AI。
</p>
<div class="tabs">
  {#each clients as c (c.key)}
    <button class:on={client === c.key} onclick={() => (client = c.key)}>{c.name}</button>
  {/each}
</div>
<pre class="cmd">{current.cmd(showToken ? token : masked)}</pre>
<div class="row small">
  <button class="link small" onclick={() => (showToken = !showToken)}>{showToken ? "隐藏 token" : "显示 token"}</button>
  {#if current.note}<span class="muted">{current.note}</span>{/if}
  <span class="spacer"></span>
  {#if copied === "fail"}<span class="muted">无法自动复制，请显示 token 后手动复制</span>{/if}
  <button class="primary" disabled={!token} onclick={copyCmd}>{copied === "ok" ? "已复制" : "复制命令"}</button>
</div>
<p class="muted small">复制的命令包含真实 token，不要贴到公开的地方。token 也在服务端 <code>data/secrets.json</code> 或环境变量 <code>GALLEY_AGENT_TOKEN</code> 中。</p>
<details class="small">
  <summary class="muted">不用 MCP？直接调 HTTP API</summary>
  <pre class="cmd">curl -H "Authorization: Bearer $GALLEY_AGENT_TOKEN" {publicUrl}/api/pending
curl -H "Authorization: Bearer $GALLEY_AGENT_TOKEN" "{publicUrl}/api/reports/&lt;id&gt;/rounds/current?format=md"</pre>
</details>
