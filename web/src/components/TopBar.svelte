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
    <span class="title" title={report.title}>{report.title}</span>
    <span class="badge">v{report.current_seq}</span>
    <nav>
      <a href="/app/r/{report.id}" class:active={active === "workbench"}>批注</a>
      <a href="/app/r/{report.id}/verify" class:active={active === "verify"}>
        验证{#if verifyCount > 0}<span class="count">&nbsp;{verifyCount}</span>{/if}
      </a>
      <a href="/app/r/{report.id}/history" class:active={active === "history"}>历史</a>
      <a href="/app/r/{report.id}/publish" class:active={active === "publish"} title={stale ? `分享的是 v${pub?.version_seq}，当前 v${report.current_seq}` : pub ? `已分享 v${pub.version_seq}` : ""}>
        发布{#if stale}<span class="count">&nbsp;有更新</span>{:else if pub}<span class="dot ok"></span>{/if}
      </a>
    </nav>
  {/if}
  <span class="spacer"></span>
  {@render children?.()}
</header>
