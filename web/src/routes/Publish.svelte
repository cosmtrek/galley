<script lang="ts">
  import { onMount } from "svelte";
  import { del, get, post } from "../lib/api";
  import { navigate } from "../lib/router.svelte";
  import { fmtTime } from "../lib/format";
  import type { Publication, ReportInfo } from "../lib/types";
  import TopBar from "../components/TopBar.svelte";
  import CopyButton from "../components/CopyButton.svelte";
  import { ask } from "../lib/confirm.svelte";

  let { id }: { id: string } = $props();

  let report = $state<ReportInfo | null>(null);
  let pubs = $state<Publication[]>([]);
  let publicUrl = $state(location.origin);
  let error = $state("");
  // Action errors are shown inside the panel the action belongs to.
  let linkError = $state("");
  let manageError = $state("");
  let busy = $state(false);

  async function load() {
    try {
      report = (await get<{ report: ReportInfo }>(`/api/reports/${id}`)).report;
      pubs = await get<Publication[]>(`/api/reports/${id}/publications`);
      publicUrl = (await get<{ public_url: string }>("/api/meta")).public_url || location.origin;
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

  async function run(setErr: (m: string) => void, action: () => Promise<unknown>) {
    busy = true;
    setErr("");
    try {
      await action();
      await load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      busy = false;
    }
  }
  const linkRun = (action: () => Promise<unknown>) => run((m) => (linkError = m), action);
  const manageRun = (action: () => Promise<unknown>) => run((m) => (manageError = m), action);

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
    linkRun(() => post(`/api/reports/${id}/publications`));
  }

  async function revoke() {
    if (!active) return;
    const ok = await ask({
      title: "停用分享链接？",
      message: "停用后这个链接永久失效，之后再发布会生成新链接。",
      confirmLabel: "停用",
      danger: true,
    });
    if (ok) linkRun(() => post(`/api/publications/${active!.id}/revoke`));
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
    manageRun(() => post(`/api/reports/${id}/archive`));
  }
  const unarchive = () => manageRun(() => post(`/api/reports/${id}/unarchive`));
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
      manageRun(async () => {
        await del(`/api/reports/${id}`);
        navigate("/app");
      });
  }
</script>

<TopBar {report} active="publish" />
{#if error}<div class="banner attention">{error}</div>{/if}

<div class="page">
  <h1>发布</h1>
  {#if report}
    {#if report.archived_at}
      <div class="banner">已归档，只读。可在下方「报告管理」中恢复。</div>
    {/if}
    {#if stale && active}
      <div class="banner attention">
        读者现在看到的还是 v{active.version_seq}，当前已经是 v{report.current_seq}。确认无误后更新分享页。
      </div>
    {/if}
    {#if pendingCount > 0}
      <div class="banner attention">
        还有 {pendingCount} 条评论未解决或待验证（草稿 {report.counts.draft} 条不计）。可以照常发布，读者看不到任何评论。
      </div>
    {/if}

    {#if active}
      <div class="panel">
        <h3>分享链接</h3>
        <div class="row share-link">
          <input type="text" readonly value={link} onfocus={(e) => (e.target as HTMLInputElement).select()} />
          <CopyButton text={link} />
          <a href={active.path} target="_blank" rel="noopener">打开 →</a>
        </div>
        <p class="small muted meta">
          已发布 v{active.version_seq} · 更新于 {fmtTime(active.updated_at)} · 打开 {active.views} 次 · 最近访问 {fmtTime(active.last_viewed_at)}
        </p>
        <p class="small muted">链接不可猜测，读者免登录；不被搜索引擎收录；页面中没有评论、修改痕迹和版本历史。</p>
        <div class="actions">
          <span class="muted small spacer">
            {active.version_id !== report.current_version_id ? "链接不变，读者刷新即可看到新版本。" : "已是当前版本，之后的修改不会影响分享页。"}
          </span>
          <button class="danger" disabled={busy} onclick={revoke}>停用链接</button>
          {#if active.version_id !== report.current_version_id}
            <button class="primary" disabled={busy} onclick={publish}>更新为当前版本 v{report.current_seq}</button>
          {/if}
        </div>
        {#if linkError}<p class="error small">{linkError}</p>{/if}
      </div>
    {:else}
      <div class="panel">
        <h3>尚未发布</h3>
        <p class="muted">发布当前版本 v{report.current_seq} 的快照，生成一个分享链接。之后的修改不会影响分享页，直到你更新发布。</p>
        <div class="actions">
          <button class="primary" disabled={busy} onclick={publish}>发布 v{report.current_seq}</button>
        </div>
        {#if linkError}<p class="error small">{linkError}</p>{/if}
      </div>
    {/if}

    {#if revoked.length}
      <h2>已停用的链接</h2>
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
    {/if}

    <h2>报告管理</h2>
    <div class="panel manage">
      {#if report.archived_at}
        <div class="row">
          <div>
            <b>已归档</b>
            <div class="muted small">归档于 {fmtTime(report.archived_at)}。恢复后可以继续评论和提交。</div>
          </div>
          <span class="spacer"></span>
          <button disabled={busy} onclick={unarchive}>恢复</button>
        </div>
        <div class="row danger">
          <div>
            <b>永久删除</b>
            <div class="muted small">删除所有版本、评论、轮次和分享记录，不可恢复。</div>
          </div>
          <span class="spacer"></span>
          <button class="danger" disabled={busy} onclick={remove}>永久删除</button>
        </div>
      {:else}
        <div class="row">
          <div>
            <b>归档</b>
            <div class="muted small">
              从列表移到「已归档」，变为只读；AI 之后交回的结果会被拒绝。分享链接继续有效。删除需要先归档。
            </div>
          </div>
          <span class="spacer"></span>
          <button disabled={busy} onclick={archive}>归档</button>
        </div>
      {/if}
      {#if manageError}<p class="error small">{manageError}</p>{/if}
    </div>
  {/if}
</div>
