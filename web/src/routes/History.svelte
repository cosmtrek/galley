<script lang="ts">
  import { onMount } from "svelte";
  import { get, post } from "../lib/api";
  import { fmtTime } from "../lib/format";
  import type { Comparison, ReportInfo, VersionMeta } from "../lib/types";
  import TopBar from "../components/TopBar.svelte";
  import PageHeader from "../components/PageHeader.svelte";
  import ChangeView from "../components/ChangeView.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import LoadError from "../components/LoadError.svelte";
  import { diffCtx, withoutPartners, type DiffCtx } from "../lib/diffctx";

  let { id }: { id: string } = $props();

  let report = $state<ReportInfo | null>(null);
  let versions = $state<VersionMeta[]>([]);
  let from = $state<string | null>(null);
  let to = $state<string | null>(null);
  let cmp = $state<Comparison | null>(null);
  let ctx = $state<DiffCtx | null>(null);
  let error = $state("");
  let loaded = $state(false);
  let comparing = $state(false);
  let compareError = $state("");
  let rollingBack = $state<string | null>(null);
  let rollbackError = $state("");
  let compareSeq = 0;

  async function load() {
    error = "";
    try {
      const [r, vs] = await Promise.all([
        get<{ report: ReportInfo }>(`/api/reports/${id}`),
        get<VersionMeta[]>(`/api/reports/${id}/versions`),
      ]);
      report = r.report;
      versions = vs;
      to = versions[0]?.id ?? null;
      from = versions[1]?.id ?? versions[0]?.id ?? null;
      loaded = true;
    } catch (e) {
      error = (e as Error).message;
      return;
    }
    await compare();
  }

  // Picking quickly can leave several comparisons in flight; only the latest one may land.
  async function compare() {
    if (!from || !to) return;
    const seq = ++compareSeq;
    comparing = true;
    compareError = "";
    try {
      const c = await get<Comparison>(`/api/reports/${id}/compare?from=${from}&to=${to}`);
      const x = await diffCtx(c.from.id, c.to.id, c.changes).catch(() => null);
      if (seq !== compareSeq) return;
      ctx = x;
      cmp = c;
    } catch (e) {
      if (seq === compareSeq) compareError = (e as Error).message;
    } finally {
      if (seq === compareSeq) comparing = false;
    }
  }

  // Rolling back adds a version rather than deleting any, so it can itself be rolled back; no confirmation.
  async function rollback(v: VersionMeta) {
    rollbackError = "";
    rollingBack = v.id;
    try {
      await post(`/api/reports/${id}/rollback`, { version_id: v.id });
      await load();
    } catch (e) {
      rollbackError = (e as Error).message;
    } finally {
      rollingBack = null;
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

<TopBar {report} loading={!report && !error} active="history" />
{#if error && loaded}<div class="banner attention">{error}</div>{/if}

{#snippet diffSkeleton()}
  <div class="skeleton" role="status" aria-label="正在加载对比">
    {#each { length: 2 }, i (i)}
      <div class="vitem" aria-hidden="true">
        <div class="vhead"><span class="sk w25"></span></div>
        <div class="vbody"><div class="diff"><Skeleton lines={3} /></div></div>
      </div>
    {/each}
  </div>
{/snippet}

<div class="page">
  <PageHeader title="版本历史" description="选两个版本对比改动，或回退到旧版本。回退会生成一个新版本，不会删除历史。" />
  {#if error && !loaded}
    <LoadError message={error} onretry={load} />
  {:else if !loaded}
    <div class="history skeleton" role="status" aria-label="正在加载版本">
      <div aria-hidden="true">
        <h2>版本</h2>
        <ul class="vlist">
          {#each { length: 4 }, i (i)}
            <li><span class="sk w40"></span><span class="sk w80 small"></span></li>
          {/each}
        </ul>
      </div>
      <div aria-hidden="true">
        <span class="sk w25 sk-lg"></span>
        {@render diffSkeleton()}
      </div>
    </div>
  {:else}
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
                  disabled={!!busy || rollingBack !== null}
                  aria-busy={rollingBack === v.id}
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
      {#if compareError}<p class="error small">{compareError}</p>{/if}
      {#if cmp}
        <div class:refreshing={comparing} aria-busy={comparing}>
          {#if cmp.changes.length === 0}
            <div class="empty">两个版本内容相同。</div>
          {/if}
          {#each withoutPartners(ctx, cmp.changes) as ch (ch.op + ch.block_id)}
            <div class="vitem">
              <div class="vhead"><span>{cmp.section_titles[ch.section_id] ?? "开头"}</span></div>
              <div class="vbody"><ChangeView change={ch} {ctx} /></div>
            </div>
          {/each}
        </div>
      {:else if comparing}
        {@render diffSkeleton()}
      {/if}
    </div>
  </div>
  {/if}
</div>
