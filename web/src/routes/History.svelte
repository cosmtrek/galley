<script lang="ts">
  import { onMount } from "svelte";
  import { get, post } from "../lib/api";
  import { fmtTime } from "../lib/format";
  import type { Comparison, ReportInfo, VersionMeta } from "../lib/types";
  import TopBar from "../components/TopBar.svelte";
  import ChangeView from "../components/ChangeView.svelte";
  import { diffCtx, withoutPartners, type DiffCtx } from "../lib/diffctx";

  let { id }: { id: string } = $props();

  let report = $state<ReportInfo | null>(null);
  let versions = $state<VersionMeta[]>([]);
  let from = $state<string | null>(null);
  let to = $state<string | null>(null);
  let cmp = $state<Comparison | null>(null);
  let ctx = $state<DiffCtx | null>(null);
  let error = $state("");
  let rollbackError = $state("");

  async function load() {
    try {
      report = (await get<{ report: ReportInfo }>(`/api/reports/${id}`)).report;
      versions = await get<VersionMeta[]>(`/api/reports/${id}/versions`);
      to = versions[0]?.id ?? null;
      from = versions[1]?.id ?? versions[0]?.id ?? null;
      await compare();
    } catch (e) {
      error = (e as Error).message;
    }
  }

  async function compare() {
    if (!from || !to) return;
    const c = await get<Comparison>(`/api/reports/${id}/compare?from=${from}&to=${to}`);
    ctx = await diffCtx(c.from.id, c.to.id, c.changes).catch(() => null);
    cmp = c;
  }

  // Rolling back adds a version rather than deleting any, so it can itself be rolled back; no confirmation.
  async function rollback(v: VersionMeta) {
    rollbackError = "";
    try {
      await post(`/api/reports/${id}/rollback`, { version_id: v.id });
      await load();
    } catch (e) {
      rollbackError = (e as Error).message;
    }
  }

  function pick(which: "from" | "to", vid: string) {
    if (which === "from") from = vid;
    else to = vid;
    compare();
  }

  onMount(load);
  const seqOf = (vid: string | null) => versions.find((v) => v.id === vid)?.seq;
</script>

<TopBar {report} active="history" />
{#if error}<div class="banner attention">{error}</div>{/if}

<div class="page wide">
  <h1>版本历史</h1>
  <div class="history">
    <div>
      <h2>版本</h2>
      {#if report?.active_round}
        <p class="muted small">第 {report.active_round.seq} 轮还没结束，结束后才能回退版本。</p>
      {/if}
      {#if rollbackError}<p class="error small">{rollbackError}</p>{/if}
      <ul class="vlist">
        {#each versions as v (v.id)}
          <li class:sel={v.id === from || v.id === to}>
            <div class="row">
              <b>v{v.seq}</b>
              <span class="muted small">{fmtTime(v.created_at)}</span>
              <span class="spacer"></span>
              {#if v.id === report?.current_version_id}<span class="badge">当前</span>{/if}
            </div>
            <div class="small">{v.note}{#if v.seq > 1}{` · ${v.changes} 处改动`}{/if}</div>
            <div class="pick">
              <label><input type="radio" name="from" checked={v.id === from} onchange={() => pick("from", v.id)} /> 旧</label>
              <label><input type="radio" name="to" checked={v.id === to} onchange={() => pick("to", v.id)} /> 新</label>
              <span class="spacer"></span>
              {#if v.id !== report?.current_version_id}
                {@const busy = report?.active_round}
                <button
                  class="link small"
                  disabled={!!busy}
                  title={busy ? `第 ${busy.seq} 轮结束后才能回退` : "用这个版本的内容生成一个新版本，未解决的评论会重新定位"}
                  onclick={() => rollback(v)}>回退到此版本</button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    </div>
    <div>
      <h2>v{seqOf(from) ?? "?"} → v{seqOf(to) ?? "?"}</h2>
      {#if cmp}
        {#if cmp.changes.length === 0}
          <div class="empty">两个版本内容相同。</div>
        {/if}
        {#each withoutPartners(ctx, cmp.changes) as ch (ch.op + ch.block_id)}
          <div class="vitem">
            <div class="vhead"><span>{cmp.section_titles[ch.section_id] ?? "开头"}</span></div>
            <div class="vbody"><ChangeView change={ch} {ctx} /></div>
          </div>
        {/each}
      {/if}
    </div>
  </div>
</div>
