<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import { get, post } from "../lib/api";
  import { fmtAgo, fmtTime } from "../lib/format";
  import { navigate } from "../lib/router.svelte";
  import type { ReportInfo } from "../lib/types";
  import TopBar from "../components/TopBar.svelte";
  import AgentConnect from "../components/AgentConnect.svelte";
  import CopyButton from "../components/CopyButton.svelte";
  import Icon from "../components/Icon.svelte";
  import PageHeader from "../components/PageHeader.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import LoadError from "../components/LoadError.svelte";
  import example from "../../../examples/energy-storage-2026.md?raw";

  interface Meta {
    public_url: string;
    agent_token: string;
    agent_last_seen: number | null;
  }

  const PROMPT = "把这份报告发到 Galley";

  let reports = $state<ReportInfo[] | null>(null);
  let error = $state("");
  let loadError = $state("");
  let creating = $state(false);
  let unarchiving = $state<string | null>(null);
  let loggingOut = $state(false);
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
    loadError = "";
    try {
      await Promise.all([loadReports(), loadMeta()]);
    } catch (e) {
      if (reports === null) loadError = (e as Error).message;
      else error = (e as Error).message;
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
    knownIds = new Set((reports ?? []).map((r) => r.id));
    addOpen = true;
    await tick();
    if (dlg && !dlg.open) dlg.showModal();
  }

  async function create() {
    creating = true;
    try {
      const r = await post<ReportInfo>("/api/reports", { markdown });
      dlg?.close();
      navigate(`/app/r/${r.id}`);
    } catch (e) {
      error = (e as Error).message;
    } finally {
      creating = false;
    }
  }

  function startExample() {
    markdown = example;
    fileName = "示例报告";
    openDialog("manual");
  }

  async function readFile(file: File | undefined): Promise<boolean> {
    if (!file) return false;
    if (!/\.(md|markdown|txt)$/i.test(file.name)) {
      error = `「${file.name}」不是 Markdown 文件，请选择 .md 文件。`;
      return false;
    }
    if (file.size > 5 * 1024 * 1024) {
      error = "文件超过 5 MB，请确认选的是报告正文。";
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
    loggingOut = true;
    try {
      await post("/api/logout");
      location.reload();
    } catch (e) {
      error = (e as Error).message;
      loggingOut = false;
    }
  }

  let filter = $state<"all" | "archived">("all");
  const active = $derived((reports ?? []).filter((r) => !r.archived_at));
  const archived = $derived((reports ?? []).filter((r) => r.archived_at));

  function setFilter(f: "all" | "archived") {
    filter = f;
    history.replaceState({}, "", f === "archived" ? "/app?f=archived" : "/app");
  }

  async function unarchive(r: ReportInfo) {
    unarchiving = r.id;
    try {
      await post(`/api/reports/${r.id}/unarchive`);
      await loadReports();
      if (archived.length === 0 && filter === "archived") setFilter("all");
    } catch (e) {
      error = (e as Error).message;
    } finally {
      unarchiving = null;
    }
  }

  /** Right-hand action shown only when the report needs the owner's attention. */
  type RowAction = { kind: "verify"; n: number } | { kind: "wait" } | { kind: "todo"; text: string };
  function actionOf(r: ReportInfo): RowAction | null {
    const ar = r.active_round;
    if (ar?.status === "verifying") return { kind: "verify", n: r.counts.verify };
    if (ar) return { kind: "wait" };
    const n = r.counts.draft + r.counts.open;
    const c = r.counts.clarify;
    if (n > 0) return { kind: "todo", text: `${n + c} 条评论待提交` };
    if (c > 0) return { kind: "todo", text: `${c} 条需要你回复` };
    return null;
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

{#snippet sendPrompt()}
  <div class="prompt-row">
    <span class="prompt">{PROMPT}</span>
    <CopyButton text={PROMPT} />
  </div>
  {#if received.length === 0}
    <p class="note"><span class="dot warn live"></span>等待 AI 发来报告…</p>
  {:else}
    {#each received as r (r.id)}
      <p class="note"><span class="dot ok"></span>已收到「{r.title}」<a href="/app/r/{r.id}">打开 →</a></p>
    {/each}
  {/if}
{/snippet}

{#snippet skeleton()}
  <div class="skeleton" role="status" aria-label="正在加载报告列表">
    <div class="page-head" aria-hidden="true"><span class="sk w25 sk-xl"></span></div>
    <ul class="reports" aria-hidden="true">
      {#each { length: 4 }, i (i)}
        <li class="report-row">
          <div class="report-main">
            <span class="sk w40 report-title"></span>
            <div class="small"><Skeleton lines={1} /></div>
            <span class="sk w25 report-meta"></span>
          </div>
        </li>
      {/each}
    </ul>
  </div>
{/snippet}

{#snippet sharePart(r: ReportInfo)}
  {@const p = r.publication}
  {#if p}
    {#if p.version_id === r.current_version_id}
      {" · 已分享"}
    {:else}
      {" · "}<span class="stale" title="读者看到的是 v{p.version_seq}">分享待更新</span>
    {/if}
  {/if}
{/snippet}

{#snippet aiFlow()}
  {#if lastSeen === null}
    <section class="step-sec">
      <h3 class="step-title">
        <span class="step-num">1</span>接入 AI
        <span class="step-status"><span class="dot warn live"></span>等待连接…</span>
      </h3>
      <AgentConnect {publicUrl} {token} />
    </section>
    <section class="step-sec">
      <h3 class="step-title"><span class="step-num">2</span>在 AI 工具里说</h3>
      {@render sendPrompt()}
    </section>
  {:else}
    <section class="step-sec">
      <h3 class="step-title">
        在 AI 工具里说
        <span class="step-status"><span class="dot ok"></span>AI 最近连接 {fmtAgo(lastSeen)}</span>
      </h3>
      {@render sendPrompt()}
    </section>
  {/if}
{/snippet}

{#snippet importForm()}
  <section class="step-sec">
    <div class="import-bar">
      <span class="muted">
        {#if fileName}已读入「{fileName}」，确认无误后创建。{:else}粘贴 Markdown，或把 .md 文件拖到这里。{/if}
      </span>
      <span class="spacer"></span>
      <button class="ghost sm" onclick={() => { markdown = example; fileName = "示例报告"; }}>填入示例</button>
      <button class="sm" onclick={() => fileInput?.click()}>选择文件</button>
    </div>
    <textarea rows="14" bind:value={markdown} placeholder={"---\ntitle: 报告标题\nsummary: 一句话摘要\n---\n\n## 一、第一章\n\n正文……"}></textarea>
  </section>
  <div class="actions">
    <span class="note spacer">创建后作为 v1。</span>
    <button class="ghost" onclick={() => dlg?.close()}>取消</button>
    <button class="primary" disabled={!markdown.trim() || creating} aria-busy={creating} onclick={create}>创建</button>
  </div>
{/snippet}

<TopBar>
  <button class="ghost" disabled={loggingOut} aria-busy={loggingOut} onclick={logout}>退出</button>
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

  {#if reports === null && loadError}
    <LoadError message={loadError} onretry={load} back={false} />
  {:else if reports === null}
    {@render skeleton()}
  {:else if reports.length === 0}
    <div class="panel empty-start">
      <h1>还没有报告</h1>
      {@render aiFlow()}
      <p class="muted alt-line">
        或者 <button class="link" onclick={() => openDialog("manual")}>手动导入</button> ·
        <button class="link" onclick={startExample}>用示例报告试试</button>
      </p>
    </div>
  {:else}
    <PageHeader title="报告">
      {#snippet actions()}
        <button class="ghost" onclick={() => openDialog("ai")}><Icon name="plus" />添加报告</button>
      {/snippet}
    </PageHeader>
    {#if archived.length > 0}
      <div class="utabs list-tabs">
        <button class:on={filter === "all"} onclick={() => setFilter("all")}>全部 {active.length}</button>
        <button class:on={filter === "archived"} onclick={() => setFilter("archived")}>已归档 {archived.length}</button>
      </div>
    {/if}
    {#if filter === "all" && active.length === 0}
      <div class="empty">没有进行中的报告。</div>
    {:else if filter === "archived"}
      <ul class="reports">
        {#each archived as r (r.id)}
          <li>
            <div class="report-row muted-row">
              <div class="report-main">
                <div class="report-head">
                  <a class="report-title" href="/app/r/{r.id}">{r.title}</a>
                  <button
                    class="sm row-action"
                    disabled={unarchiving !== null}
                    aria-busy={unarchiving === r.id}
                    onclick={() => unarchive(r)}>恢复</button
                  >
                </div>
                {#if r.summary}<div class="muted small clamp">{r.summary}</div>{/if}
                <div class="muted report-meta">归档于 {fmtTime(r.archived_at)} · v{r.current_seq}{@render sharePart(r)}</div>
              </div>
            </div>
          </li>
        {/each}
      </ul>
    {:else}
      <ul class="reports">
        {#each active as r (r.id)}
          {@const a = actionOf(r)}
          <li>
            <div class="report-row">
              <div class="report-main">
                <div class="report-head">
                  <a class="report-title" href="/app/r/{r.id}">{r.title}</a>
                  {#if a?.kind === "verify"}
                    <a class="btn primary sm" href="/app/r/{r.id}">待验证 {a.n} 条 →</a>
                  {:else if a?.kind === "wait"}
                    <span class="act-wait"><span class="dot warn live"></span>等待 AI</span>
                  {:else if a?.kind === "todo"}
                    <span class="act-todo">{a.text}</span>
                  {/if}
                </div>
                {#if r.summary}<div class="muted small clamp">{r.summary}</div>{/if}
                <div class="muted report-meta">{fmtTime(r.updated_at)} · v{r.current_seq}{@render sharePart(r)}</div>
              </div>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<dialog
  class="modal add"
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
    <h2>添加报告</h2>
    <span class="spacer"></span>
    <button class="ghost sm icon" onclick={() => dlg?.close()} aria-label="关闭" title="关闭"><Icon name="x" /></button>
  </div>
  <div class="utabs dlg-tabs">
    <button class:on={addTab === "ai"} onclick={() => (addTab = "ai")}>让 AI 发过来</button>
    <button class:on={addTab === "manual"} onclick={() => (addTab = "manual")}>手动导入</button>
  </div>
  {#if error && addOpen}<p class="error small">{error}</p>{/if}
  <div class="dlg-body">
    {#if addTab === "ai"}
      {@render aiFlow()}
      {#if lastSeen !== null}
        <details class="dlg-foot" bind:open={setupOpen}>
          <summary>接入方法 / 换个客户端</summary>
          <div class="step-sec"><AgentConnect {publicUrl} {token} /></div>
        </details>
      {/if}
    {:else}
      {@render importForm()}
    {/if}
  </div>
</dialog>
