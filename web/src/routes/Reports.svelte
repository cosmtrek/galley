<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import { get, post } from "../lib/api";
  import { copyText } from "../lib/clipboard";
  import { fmtAgo, fmtTime, ROUND_LABEL } from "../lib/format";
  import { navigate } from "../lib/router.svelte";
  import type { ReportInfo } from "../lib/types";
  import TopBar from "../components/TopBar.svelte";
  import AgentConnect from "../components/AgentConnect.svelte";
  import example from "../../../examples/energy-storage-2026.md?raw";

  interface Meta {
    public_url: string;
    agent_token: string;
    agent_last_seen: number | null;
  }

  const PROMPT = "把这份报告发到 Galley";

  let reports = $state<ReportInfo[] | null>(null);
  let error = $state("");
  let markdown = $state("");
  let fileName = $state("");
  let dragging = $state(false);
  let fileInput = $state<HTMLInputElement | null>(null);
  let publicUrl = $state(location.origin);
  let token = $state("");
  let lastSeen = $state<number | null>(null);

  let dlg = $state<HTMLDialogElement | null>(null);
  let addOpen = $state(false);
  let addTab = $state<"ai" | "manual">("ai");
  let setupOpen = $state(false);
  let promptCopied = $state<"" | "ok" | "fail">("");
  // Ids present when the dialog opened; anything newer arrived from the agent.
  let knownIds = new Set<string>();
  const received = $derived(addOpen ? (reports ?? []).filter((r) => !knownIds.has(r.id)) : []);

  async function loadMeta() {
    const meta = await get<Meta>("/api/meta");
    publicUrl = meta.public_url || location.origin;
    token = meta.agent_token;
    lastSeen = meta.agent_last_seen;
  }

  async function loadReports() {
    reports = await get<ReportInfo[]>("/api/reports");
  }

  async function load() {
    try {
      await loadReports();
      await loadMeta();
    } catch (e) {
      error = (e as Error).message;
      return;
    }
    const f = new URLSearchParams(location.search).get("f");
    filter = f === "archived" && archived.length > 0 ? "archived" : "all";
    if (location.hash === "#agent") openDialog("ai", true);
  }

  async function openDialog(tab: "ai" | "manual", setup = false) {
    addTab = tab;
    setupOpen = setup;
    error = "";
    promptCopied = "";
    knownIds = new Set((reports ?? []).map((r) => r.id));
    addOpen = true;
    await tick();
    if (dlg && !dlg.open) dlg.showModal();
  }

  async function create() {
    try {
      const r = await post<ReportInfo>("/api/reports", { markdown });
      dlg?.close();
      navigate(`/app/r/${r.id}`);
    } catch (e) {
      error = (e as Error).message;
    }
  }

  async function copyPrompt() {
    promptCopied = (await copyText(PROMPT)) ? "ok" : "fail";
    setTimeout(() => (promptCopied = ""), 2000);
  }

  function startExample() {
    markdown = example;
    fileName = "示例报告";
    openDialog("manual");
  }

  async function readFile(file: File | undefined): Promise<boolean> {
    if (!file) return false;
    if (!/\.(md|markdown|txt)$/i.test(file.name)) {
      error = `「${file.name}」不是 Markdown 文件，请选择 .md 文件`;
      return false;
    }
    if (file.size > 5 * 1024 * 1024) {
      error = "文件超过 5 MB，请确认选的是报告正文";
      return false;
    }
    markdown = await file.text();
    fileName = file.name;
    error = "";
    return true;
  }

  async function onDrop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    if (await readFile(e.dataTransfer?.files[0])) openDialog("manual");
  }
  function onDragOver(e: DragEvent) {
    e.preventDefault();
    dragging = true;
  }

  async function logout() {
    await post("/api/logout");
    location.reload();
  }

  let filter = $state<"all" | "archived">("all");
  const active = $derived((reports ?? []).filter((r) => !r.archived_at));
  const archived = $derived((reports ?? []).filter((r) => r.archived_at));

  function setFilter(f: "all" | "archived") {
    filter = f;
    history.replaceState({}, "", f === "archived" ? "/app?f=archived" : "/app");
  }

  async function unarchive(r: ReportInfo) {
    try {
      await post(`/api/reports/${r.id}/unarchive`);
      await loadReports();
      if (archived.length === 0 && filter === "archived") setFilter("all");
    } catch (e) {
      error = (e as Error).message;
    }
  }

  /** How the share link relates to the current draft, if there is one. */
  function share(r: ReportInfo): { text: string; stale: boolean } | null {
    const p = r.publication;
    if (!p) return null;
    if (p.version_id !== r.current_version_id) return { text: `分享的是 v${p.version_seq}，当前 v${r.current_seq}`, stale: true };
    return { text: `已分享 v${p.version_seq} · 打开 ${p.views} 次`, stale: false };
  }

  /** One sentence per report: what, if anything, the owner should do next. */
  function status(r: ReportInfo): { text: string; tone: "verify" | "wait" | "todo" | "" } {
    const ar = r.active_round;
    if (ar?.status === "verifying") return { text: `第 ${ar.seq} 轮 · ${ROUND_LABEL.verifying} ${r.counts.verify} 条`, tone: "verify" };
    if (ar) return { text: `第 ${ar.seq} 轮 · ${ROUND_LABEL[ar.status]}`, tone: "wait" };
    const pending = r.counts.draft + r.counts.open + r.counts.clarify;
    if (pending) return { text: `${pending} 条评论待交给 AI`, tone: "todo" };
    return { text: "没有待处理", tone: "" };
  }

  // Poll while there is something live to wait for: agent's first connection on the
  // empty state, or new reports / the first connection while the dialog is open.
  let poll: ReturnType<typeof setInterval> | undefined;
  $effect(() => {
    clearInterval(poll);
    if (addOpen || (reports !== null && reports.length === 0)) {
      poll = setInterval(() => {
        loadReports().catch(() => {});
        if (lastSeen === null) loadMeta().catch(() => {});
      }, 4000);
    }
  });
  onDestroy(() => clearInterval(poll));

  onMount(load);
</script>

{#snippet promptRow()}
  <div class="prompt-row">
    <span class="prompt">{PROMPT}</span>
    <button class="primary" onclick={copyPrompt}>{promptCopied === "ok" ? "已复制" : "复制"}</button>
  </div>
  {#if promptCopied === "fail"}<p class="muted small">复制失败，请选中后手动复制</p>{/if}
{/snippet}

{#snippet receivedArea()}
  {#if received.length === 0}
    <p class="muted small">⟳ 等待 AI 发来报告…</p>
  {:else}
    {#each received as r (r.id)}
      <p class="small received">✓ 已收到「{r.title}」 <a href="/app/r/{r.id}">打开</a></p>
    {/each}
  {/if}
{/snippet}

{#snippet aiFlow(inline: boolean)}
  {#if lastSeen === null}
    <div class="steps">
      <div class="step-block">
        <p class="small step">
          <b>① 接入 AI</b>
          <span class="muted">⟳ 等待连接…</span>
        </p>
        <div><AgentConnect {publicUrl} {token} /></div>
      </div>
      <div class="step-block">
        <p class="small step"><b>② 让 AI 把报告发过来</b></p>
        <div class="sec">
          <p class="small">在 AI 工具里说：</p>
          {@render promptRow()}
          {@render receivedArea()}
        </div>
      </div>
    </div>
  {:else}
    <div class="sec">
      {#if inline}<p class="muted small">✓ 已连接 · 最近 {fmtAgo(lastSeen)}</p>{/if}
      <p class="small">在 AI 工具里说：</p>
      {@render promptRow()}
      {@render receivedArea()}
    </div>
  {/if}
{/snippet}

{#snippet importForm()}
  <div class="sec">
    <div class="row">
      <button onclick={() => fileInput?.click()}>选择 .md 文件</button>
      <button class="quiet" onclick={() => { markdown = example; fileName = "示例报告"; }}>填入示例报告</button>
    </div>
    <p class="muted small">
      {#if fileName}已读入「{fileName}」，确认无误后创建。{:else}把 .md 文件拖到这里、选择文件，或直接粘贴 Markdown，创建后作为 v1。{/if}
    </p>
  </div>
  <textarea rows="12" bind:value={markdown} placeholder={"---\ntitle: 报告标题\nsummary: 一句话摘要\n---\n\n## 一、第一章\n\n正文……"}></textarea>
  <div class="row">
    <button class="primary" disabled={!markdown.trim()} onclick={create}>创建</button>
    <button onclick={() => dlg?.close()}>取消</button>
  </div>
{/snippet}

<TopBar>
  <button class="quiet" onclick={logout}>退出</button>
</TopBar>

<input
  type="file"
  accept=".md,.markdown,.txt,text/markdown,text/plain"
  hidden
  bind:this={fileInput}
  onchange={async (e) => {
    const input = e.currentTarget as HTMLInputElement;
    if (await readFile(input.files?.[0])) openDialog("manual");
    input.value = "";
  }}
/>

<div
  class="page"
  class:dragging
  role="region"
  aria-label="报告列表"
  ondragover={onDragOver}
  ondragleave={() => (dragging = false)}
  ondrop={onDrop}
>
  {#if error && !addOpen}<p class="error">{error}</p>{/if}

  {#if reports === null}
    <p class="muted">加载中…</p>
  {:else if reports.length === 0}
    <div class="panel empty-start">
      <h1>还没有报告</h1>
      {@render aiFlow(true)}
      <p class="muted small alt-line">
        或者 <button class="link" onclick={() => openDialog("manual")}>手动导入</button> ·
        <button class="link" onclick={startExample}>用示例报告试试</button>
      </p>
    </div>
  {:else}
    <div class="row list-head">
      <h1>报告</h1>
      <span class="spacer"></span>
      <button class="primary add-btn" onclick={() => openDialog("ai")}>+ 添加报告</button>
    </div>
    <p class="muted list-sub">
      {active.length} 篇 · <span class="conn-dot" class:on={lastSeen !== null}></span>
      {#if lastSeen === null}AI 从未连接{:else}AI 最近连接 {fmtAgo(lastSeen)}{/if} ·
      <button class="link" onclick={() => openDialog("ai", true)}>接入方法</button>
    </p>
    {#if archived.length > 0}
      <div class="utabs">
        <button class:on={filter === "all"} onclick={() => setFilter("all")}>全部 {active.length}</button>
        <button class:on={filter === "archived"} onclick={() => setFilter("archived")}>已归档 {archived.length}</button>
      </div>
    {/if}
    {#if filter === "all" && active.length === 0}
      <p class="muted">没有进行中的报告</p>
    {:else if filter === "archived"}
      <ul class="reports">
        {#each archived as r (r.id)}
          {@const sh = share(r)}
          <li>
            <div class="report-row muted-row">
              <a class="report-main" href="/app/r/{r.id}">
                <div class="report-title">{r.title}</div>
                <div class="muted small report-meta">
                  <span>v{r.current_seq}</span>
                  <span>· 归档于 {fmtTime(r.archived_at)}</span>
                  {#if sh}<span>· {sh.text}</span>{/if}
                </div>
              </a>
              <button class="small" onclick={() => unarchive(r)}>恢复</button>
            </div>
          </li>
        {/each}
      </ul>
    {:else}
      <ul class="reports">
        {#each active as r (r.id)}
          {@const s = status(r)}
          {@const sh = share(r)}
          <li>
            <a class="report-row bar-{s.tone || "none"}" href="/app/r/{r.id}">
              <div class="report-main">
                <div class="report-title">{r.title}</div>
                {#if r.summary}<div class="muted small clamp">{r.summary}</div>{/if}
                <div class="muted small report-meta">
                  <span>v{r.current_seq}</span>
                  <span>· {fmtTime(r.updated_at)}</span>
                  {#if sh}<span class:stale={sh.stale}>· {sh.text}</span>{/if}
                </div>
              </div>
              <div class="report-status tone-{s.tone}">{s.text}</div>
            </a>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<dialog
  class="add"
  bind:this={dlg}
  onclose={() => (addOpen = false)}
  onclick={(e) => {
    if (e.target === dlg) dlg?.close();
  }}
  ondragover={onDragOver}
  ondragleave={() => (dragging = false)}
  ondrop={onDrop}
>
  <div class="row dlg-head">
    <h2 style="margin: 0">添加报告</h2>
    <span class="spacer"></span>
    <button class="quiet dlg-close" onclick={() => dlg?.close()} aria-label="关闭">✕</button>
  </div>
  <div class="utabs dlg-tabs">
    <button class:on={addTab === "ai"} onclick={() => (addTab = "ai")}>让 AI 发过来</button>
    <button class:on={addTab === "manual"} onclick={() => (addTab = "manual")}>手动导入</button>
  </div>
  {#if error && addOpen}<p class="error small">{error}</p>{/if}
  <div class="dlg-body">
    {#if addTab === "ai"}
      {@render aiFlow(false)}
      {#if lastSeen !== null}
        <div class="row dlg-foot">
          <span class="muted small"><span class="conn-dot on"></span> 最近连接 {fmtAgo(lastSeen)}</span>
          <span class="spacer"></span>
          <button class="link small" onclick={() => (setupOpen = !setupOpen)}>接入方法 / 换个客户端</button>
        </div>
        {#if setupOpen}
          <div><AgentConnect {publicUrl} {token} /></div>
        {/if}
      {/if}
    {:else}
      {@render importForm()}
    {/if}
  </div>
</dialog>
