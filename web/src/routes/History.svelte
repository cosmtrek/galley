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

  async function rollback(v: VersionMeta) {
    if (!confirm(`以 v${v.seq} 的内容生成一个新版本？未解决的评论会重新定位到新版本。`)) return;
    try {
      await post(`/api/reports/${id}/rollback`, { version_id: v.id });
      await load();
    } catch (e) {
      alert((e as Error).message);
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

<div class="page" style="max-width: 1200px">
  <div class="history">
    <div>
      <h1 style="font-size: 18px">版本</h1>
      {#if report?.active_round}
        <p class="muted small">第 {report.active_round.seq} 轮还没结束，结束后才能回退版本。</p>
      {/if}
      <ul class="vlist">
        {#each versions as v (v.id)}
          <li class:sel={v.id === from || v.id === to}>
            <div class="row">
              <b>v{v.seq}</b>
              <span class="muted small">{fmtTime(v.created_at)}</span>
              <span class="spacer"></span>
              {#if v.id === report?.current_version_id}<span class="badge">当前</span>{/if}
            </div>
            <div class="small">{v.note}{#if v.seq > 1} · {v.changes} 处改动{/if}</div>
            <div class="pick">
              <label><input type="radio" name="from" checked={v.id === from} onchange={() => pick("from", v.id)} /> 旧</label>
              <label><input type="radio" name="to" checked={v.id === to} onchange={() => pick("to", v.id)} /> 新</label>
              <span class="spacer"></span>
              {#if v.id !== report?.current_version_id}
                {@const busy = report?.active_round}
                <button
                  class="link small"
                  disabled={!!busy}
                  title={busy ? `第 ${busy.seq} 轮结束后才能回退` : ""}
                  onclick={() => rollback(v)}>回退到此版本</button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    </div>
    <div>
      <h1 style="font-size: 18px">v{seqOf(from) ?? "?"} → v{seqOf(to) ?? "?"}</h1>
      {#if cmp}
        {#if cmp.changes.length === 0}
          <p class="muted">两个版本内容相同。</p>
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
