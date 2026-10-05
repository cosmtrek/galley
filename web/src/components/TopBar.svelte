<script lang="ts">
  import type { Snippet } from "svelte";
  import type { ReportInfo } from "../lib/types";

  let {
    report = null,
    active = "",
    children,
  }: { report?: ReportInfo | null; active?: string; children?: Snippet } = $props();

  const verifyCount = $derived(report ? report.counts.verify : 0);
  const pub = $derived(report?.publication ?? null);
  const stale = $derived(!!pub && !!report && pub.version_id !== report.current_version_id);
</script>

<header class="topbar">
  <a class="brand" href="/app">Galley</a>
  {#if report}
    <span class="muted">/</span>
    {#if active === "workbench"}
      <span class="title" aria-current="page" title={report.title}>{report.title}</span>
    {:else}
      <a class="title" href="/app/r/{report.id}" title="回到正文">{report.title}</a>
    {/if}
    <a
      class="badge vlink"
      href="/app/r/{report.id}/history"
      title="版本历史"
      class:active={active === "history"}
      aria-current={active === "history" ? "page" : undefined}>v{report.current_seq}</a
    >
  {/if}
  <span class="spacer"></span>
  {#if report}
    {#if report.active_round?.status === "verifying" && active !== "workbench"}
      <a class="verify-chip" href="/app/r/{report.id}">
        第 {report.active_round.seq} 轮待验证{#if verifyCount > 0}&nbsp;· {verifyCount}{/if}
      </a>
    {/if}
    <a
      class="pub-btn"
      href="/app/r/{report.id}/publish"
      class:active={active === "publish"}
      title={stale ? `分享的是 v${pub?.version_seq}，当前 v${report.current_seq}` : pub ? `已分享 v${pub.version_seq}` : ""}
    >
      发布{#if stale}<span class="stale">&nbsp;· 有更新</span>{/if}
    </a>
  {/if}
  {@render children?.()}
</header>
