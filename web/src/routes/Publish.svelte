<script lang="ts">
  import { onMount } from "svelte";
  import { get, post } from "../lib/api";
  import { fmtTime } from "../lib/format";
  import type { Publication, ReportInfo } from "../lib/types";
  import TopBar from "../components/TopBar.svelte";

  let { id }: { id: string } = $props();

  let report = $state<ReportInfo | null>(null);
  let pubs = $state<Publication[]>([]);
  let publicUrl = $state(location.origin);
  let error = $state("");
  let copied = $state(false);
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

  async function publish() {
    if (pendingCount > 0 && !confirm(`还有 ${pendingCount} 条评论未解决或待验证，仍然发布 v${report?.current_seq}？`)) return;
    busy = true;
    try {
      await post(`/api/reports/${id}/publications`);
      await load();
    } catch (e) {
      alert((e as Error).message);
    } finally {
      busy = false;
    }
  }

  async function revoke() {
    if (!active || !confirm("撤销后这个链接将永久失效（返回 410），之后再发布会生成新链接。")) return;
    await post(`/api/publications/${active.id}/revoke`);
    await load();
  }

  async function copy() {
    await navigator.clipboard.writeText(link);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<TopBar {report} active="publish" />
{#if error}<div class="banner attention">{error}</div>{/if}

<div class="page">
  <h1>发布</h1>
  {#if report}
    {#if pendingCount > 0}
      <div class="banner attention" style="margin-bottom: 16px">
        还有 {pendingCount} 条评论未解决或待验证（草稿 {report.counts.draft} 条不计）。可以照常发布，读者看不到任何评论。
      </div>
    {/if}

    {#if active}
      <div class="panel">
        <h3>分享链接</h3>
        <div class="row">
          <input type="text" readonly value={link} onfocus={(e) => (e.target as HTMLInputElement).select()} />
          <button onclick={copy}>{copied ? "已复制" : "复制"}</button>
          <a href={active.path} target="_blank" rel="noopener">打开</a>
        </div>
        <p class="small muted" style="margin-top: 12px">
          已发布 v{active.version_seq} · 更新于 {fmtTime(active.updated_at)} · 打开 {active.views} 次 · 最近访问 {fmtTime(active.last_viewed_at)}
        </p>
        <p class="small muted">链接不可猜测，读者免登录；不被搜索引擎收录；页面中没有评论、修改痕迹和版本历史。</p>
        <div class="row" style="margin-top: 12px">
          {#if active.version_id !== report.current_version_id}
            <button class="primary" disabled={busy} onclick={publish}>更新为当前版本 v{report.current_seq}</button>
            <span class="muted small">链接不变，读者刷新即可看到新版本。</span>
          {:else}
            <span class="muted small">已是当前版本。草稿继续修改不会影响分享页。</span>
          {/if}
          <span class="spacer"></span>
          <button onclick={revoke}>撤销链接</button>
        </div>
      </div>
    {:else}
      <div class="panel">
        <h3>尚未发布</h3>
        <p class="muted">发布当前版本 v{report.current_seq} 的快照，生成一个分享链接。之后草稿的修改不会影响分享页，直到你更新发布。</p>
        <button class="primary" disabled={busy} onclick={publish}>发布 v{report.current_seq}</button>
      </div>
    {/if}

    {#if revoked.length}
      <h2>已撤销的链接</h2>
      <table class="list">
        <thead><tr><th>链接</th><th>版本</th><th>打开次数</th><th>撤销时间</th></tr></thead>
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
  {/if}
</div>
