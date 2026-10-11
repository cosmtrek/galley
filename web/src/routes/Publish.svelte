<script lang="ts">
  import { onMount } from "svelte";
  import { del, get, post } from "../lib/api";
  import { navigate } from "../lib/router.svelte";
  import { fmtTime } from "../lib/format";
  import type { Publication, ReportInfo } from "../lib/types";
  import TopBar from "../components/TopBar.svelte";
  import CopyButton from "../components/CopyButton.svelte";
  import Icon from "../components/Icon.svelte";
  import PageHeader from "../components/PageHeader.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import LoadError from "../components/LoadError.svelte";
  import { ask } from "../lib/confirm.svelte";

  let { id }: { id: string } = $props();

  let report = $state<ReportInfo | null>(null);
  let pubs = $state<Publication[]>([]);
  let publicUrl = $state(location.origin);
  let error = $state("");
  // Action errors are shown inside the panel the action belongs to.
  let linkError = $state("");
  let manageError = $state("");
  let dangerError = $state("");
  type Action = "publish" | "archive" | "unarchive" | "revoke" | "delete";
  // The running action: its button shows the spinner, every other action waits.
  let busy = $state<Action | null>(null);

  async function load() {
    error = "";
    try {
      const [r, ps, meta] = await Promise.all([
        get<{ report: ReportInfo }>(`/api/reports/${id}`),
        get<Publication[]>(`/api/reports/${id}/publications`),
        get<{ public_url: string }>("/api/meta"),
      ]);
      report = r.report;
      pubs = ps;
      publicUrl = meta.public_url || location.origin;
    } catch (e) {
      error = (e as Error).message;
    }
  }
  onMount(load);

  const active = $derived(pubs.find((p) => p.revoked_at === null) ?? null);
  const revoked = $derived(pubs.filter((p) => p.revoked_at !== null));
  const pendingCount = $derived(
    report ? report.counts.open + report.counts.clarify + report.counts.verify + report.counts.orphaned : 0,
  );
  const link = $derived(active ? `${publicUrl.replace(/\/$/, "")}${active.path}` : "");

  async function run(kind: Action, setErr: (m: string) => void, action: () => Promise<unknown>) {
    busy = kind;
    setErr("");
    try {
      await action();
      await load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      busy = null;
    }
  }
  const linkRun = (kind: Action, action: () => Promise<unknown>) => run(kind, (m) => (linkError = m), action);
  const manageRun = (kind: Action, action: () => Promise<unknown>) => run(kind, (m) => (manageError = m), action);
  const dangerRun = (kind: Action, action: () => Promise<unknown>) => run(kind, (m) => (dangerError = m), action);

  async function publish() {
    if (
      pendingCount > 0 &&
      !(await ask({
        title: `发布 v${report?.current_seq}？`,
        message: `还有 ${pendingCount} 条评论未解决或待验证。读者看不到评论，只会看到当前正文。`,
        confirmLabel: "仍然发布",
      }))
    )
      return;
    linkRun("publish", () => post(`/api/reports/${id}/publications`));
  }

  async function revoke() {
    if (!active) return;
    const ok = await ask({
      title: "停用分享链接？",
      message: "停用后这个链接永久失效，之后再发布会生成新链接。",
      confirmLabel: "停用",
      danger: true,
    });
    if (ok) dangerRun("revoke", () => post(`/api/publications/${active!.id}/revoke`));
  }

  // ----- report management -----

  const stale = $derived(!!active && !!report && active.version_id !== report.current_version_id);

  // Archiving is reversible, so it only asks when it would throw away a round the AI is working on.
  async function archive() {
    const ar = report?.active_round;
    if (
      ar &&
      ar.status !== "verifying" &&
      !(await ask({
        title: "归档报告？",
        message: `第 ${ar.seq} 轮正在等 AI，归档后它交回的结果会作废，恢复后需要让 AI 重做。`,
        confirmLabel: "归档",
      }))
    )
      return;
    manageRun("archive", () => post(`/api/reports/${id}/archive`));
  }
  const unarchive = () => manageRun("unarchive", () => post(`/api/reports/${id}/unarchive`));
  async function remove() {
    if (!report) return;
    const warn = active ? `分享链接会立刻失效（已打开 ${active.views} 次）。` : "";
    const ok = await ask({
      title: "永久删除报告？",
      message: `所有版本、评论和轮次记录都会被清除，无法恢复。${warn}`,
      confirmLabel: "永久删除",
      danger: true,
      requireText: report.title,
    });
    if (ok)
      dangerRun("delete", async () => {
        await del(`/api/reports/${id}`);
        navigate("/app");
      });
  }
</script>

<TopBar {report} loading={!report && !error} active="publish" />
{#if error && report}<div class="banner attention">{error}</div>{/if}

<div class="page">
  <PageHeader title="发布" description="把报告的某个版本发布成只读分享页，读者打开链接即可阅读，不用登录。" />
  {#if error && !report}
    <LoadError message={error} onretry={load} />
  {:else if !report}
    <div class="skeleton" role="status" aria-label="正在加载发布状态">
      <section class="panel stack" aria-hidden="true">
        <div class="panel-head">
          <div class="grow">
            <span class="sk w40 status-title"></span>
            <div class="small"><Skeleton lines={1} /></div>
          </div>
        </div>
        <span class="sk w80 small"></span>
      </section>
      <h2>报告管理</h2>
      <section class="panel settings" aria-hidden="true">
        <div class="setting"><div><span class="sk w25"></span><span class="sk w60 small"></span></div></div>
      </section>
    </div>
  {:else}
    {#if report.archived_at}
      <div class="banner">已归档，只读。可在下方「报告管理」中恢复。</div>
    {/if}

    <section class="panel stack">
      <div class="panel-head">
        <div class="grow">
          <div class="status-title">
            <span class="dot" class:ok={active && !stale} class:warn={stale}></span>
            {#if !active}
              尚未发布
            {:else if stale}
              分享页是 v{active.version_seq}，当前已是 v{report.current_seq}
            {:else}
              已发布 v{active.version_seq}，是当前版本
            {/if}
          </div>
          <div class="muted small">
            {#if !active}
              发布 v{report.current_seq} 的快照并生成分享链接。之后的修改不会影响分享页，直到你更新发布。
            {:else if stale}
              更新后链接不变，读者刷新即可看到 v{report.current_seq}。
            {:else}
              之后的修改不会影响分享页，直到你更新发布。
            {/if}
          </div>
        </div>
        {#if !active || stale}
          <button class="primary" disabled={!!busy} aria-busy={busy === "publish"} onclick={publish}>
            {active ? `更新为 v${report.current_seq}` : `发布 v${report.current_seq}`}
          </button>
        {/if}
      </div>
      {#if pendingCount > 0 && (!active || stale)}
        <p class="note"><span class="dot warn"></span>还有 {pendingCount} 条评论未解决或待验证（草稿不计）。可以照常发布，读者看不到任何评论。</p>
      {/if}

      {#if active}
        <div class="field">
          <span class="field-label">分享链接</span>
          <div class="row share-link">
            <input type="text" readonly value={link} onfocus={(e) => (e.target as HTMLInputElement).select()} />
            <CopyButton text={link} />
            <a class="btn" href={active.path} target="_blank" rel="noopener"><Icon name="external" />打开</a>
          </div>
        </div>
        <dl class="stats">
          <div><dt>打开次数</dt><dd>{active.views}</dd></div>
          <div><dt>最近访问</dt><dd>{fmtTime(active.last_viewed_at)}</dd></div>
          <div><dt>更新于</dt><dd>{fmtTime(active.updated_at)}</dd></div>
        </dl>
      {/if}
      <p class="muted small panel-foot">链接不可猜测，读者免登录，不被搜索引擎收录；分享页不含评论、修改痕迹和版本历史。</p>
      {#if linkError}<p class="error small">{linkError}</p>{/if}
    </section>

    {#if revoked.length}
      <details class="revoked">
        <summary>已停用的链接 · {revoked.length}</summary>
        <table class="list">
          <thead><tr><th>链接</th><th>版本</th><th>打开次数</th><th>停用时间</th></tr></thead>
          <tbody>
            {#each revoked as p (p.id)}
              <tr>
                <td class="mono">{p.path}</td>
                <td>v{p.version_seq}</td>
                <td>{p.views}</td>
                <td class="muted">{fmtTime(p.revoked_at)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </details>
    {/if}

    <h2>报告管理</h2>
    <section class="panel settings">
      <div class="setting">
        <div>
          <b>{report.archived_at ? "已归档" : "归档"}</b>
          <span class="muted small">
            {#if report.archived_at}
              归档于 {fmtTime(report.archived_at)}。恢复后可以继续评论和提交。
            {:else}
              从列表移到「已归档」并变为只读，AI 之后交回的结果会被拒绝。分享链接继续有效。
            {/if}
          </span>
        </div>
        {#if report.archived_at}
          <button disabled={!!busy} aria-busy={busy === "unarchive"} onclick={unarchive}>恢复</button>
        {:else}
          <button disabled={!!busy} aria-busy={busy === "archive"} onclick={archive}>归档</button>
        {/if}
      </div>
      {#if manageError}<p class="error small setting-error">{manageError}</p>{/if}
    </section>

    <h2>不可撤销的操作</h2>
    <section class="panel settings">
      {#if active}
        <div class="setting">
          <div>
            <b>停用分享链接</b>
            <span class="muted small">链接永久失效（已打开 {active.views} 次），之后再发布会生成新链接。</span>
          </div>
          <button class="danger" disabled={!!busy} aria-busy={busy === "revoke"} onclick={revoke}>停用链接</button>
        </div>
      {/if}
      <div class="setting">
        <div>
          <b>永久删除报告</b>
          <span class="muted small">
            删除所有版本、评论、轮次和分享记录。{report.archived_at ? "" : "需要先归档。"}
          </span>
        </div>
        <button class="danger" disabled={!!busy || !report.archived_at} aria-busy={busy === "delete"} onclick={remove}>永久删除</button>
      </div>
      {#if dangerError}<p class="error small setting-error">{dangerError}</p>{/if}
    </section>
  {/if}
</div>
