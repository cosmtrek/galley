<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { computePosition, flip, offset, shift } from "@floating-ui/dom";
  import { api, get, post } from "../lib/api";
  import { anchorToRange, scopeFor, selectionToAnchor } from "../lib/anchor";
  import { STATUS_LABEL, truncate } from "../lib/format";
  import type { Anchor, Comment, CommentStatus, ReportInfo, Round, Version } from "../lib/types";
  import TopBar from "../components/TopBar.svelte";
  import Outline from "../components/Outline.svelte";
  import CommentCard from "../components/CommentCard.svelte";
  import Composer from "../components/Composer.svelte";

  let { id }: { id: string } = $props();

  type Mode = "read" | "comment";
  type Filter = "unresolved" | "all" | CommentStatus;
  type Ref = { getBoundingClientRect(): DOMRect };
  type Popover =
    | { kind: "compose"; anchor: Anchor; label: string | null; ref: Ref; range: Range | null }
    | { kind: "view"; commentId: string; ref: Ref };

  const MODE_KEY = "galley.mode";
  const STATUS_ORDER: CommentStatus[] = ["draft", "open", "clarify", "verify", "orphaned"];
  const HIGHLIGHTS = ["galley-comment", "galley-draft", "galley-active", "galley-pending"];

  let report = $state<ReportInfo | null>(null);
  let version = $state<Version | null>(null);
  let comments = $state<Comment[]>([]);
  let error = $state("");
  let mode = $state<Mode>(localStorage.getItem(MODE_KEY) === "read" ? "read" : "comment");
  let filter = $state<Filter>("unresolved");
  let activeId = $state<string | null>(null);
  let pop = $state.raw<Popover | null>(null);
  let composerDirty = $state(false);
  let gutter = $state<{ top: number; blockId: string; heading: boolean } | null>(null);
  let submitting = $state(false);
  let batchBusy = $state(false);
  const picked = new SvelteSet<string>();

  let articleEl = $state<HTMLElement | null>(null);
  let paperEl = $state<HTMLElement | null>(null);
  let titleEl = $state<HTMLElement | null>(null);
  let popEl = $state<HTMLElement | null>(null);
  let allBox = $state<HTMLInputElement | null>(null);
  let ranges = new Map<string, Range>();
  let poll: ReturnType<typeof setInterval> | undefined;

  const commenting = $derived(mode === "comment");
  const titles = $derived(
    new Map((version?.doc.blocks ?? []).filter((b) => b.kind === "heading").map((b) => [b.id, b.text])),
  );
  const statusCounts = $derived.by(() => {
    const m = new Map<CommentStatus, number>();
    for (const c of comments) m.set(c.status, (m.get(c.status) ?? 0) + 1);
    return m;
  });
  const unresolvedCount = $derived(comments.length - (statusCounts.get("resolved") ?? 0));
  const shown = $derived(
    comments.filter((c) =>
      filter === "all" ? true : filter === "unresolved" ? c.status !== "resolved" : c.status === filter,
    ),
  );
  const submittable = $derived(comments.filter((c) => c.status === "draft" || c.status === "open").length);
  const drafts = $derived(statusCounts.get("draft") ?? 0);
  const round = $derived<Round | null>(report?.active_round ?? null);
  const roundBusy = $derived(round !== null);
  const agentPrompt = $derived(
    round && report
      ? `处理 Galley 报告「${report.title}」第 ${round.seq} 轮评论（report_id: ${report.id}）：用 galley MCP 的 galley_get_round 读取评论，按评论修改后用 galley_submit_round 一次性提交新 Markdown、逐条回复和本轮摘要。`
      : "",
  );
  let copied = $state(false);
  let roundOpen = $state(false);
  let roundEl = $state<HTMLElement | null>(null);
  let now = $state(Date.now());

  function ago(ms: number | null) {
    if (!ms) return "";
    const min = Math.floor((now - ms) / 60000);
    if (min < 1) return "刚刚";
    if (min < 60) return `${min} 分钟前`;
    return `${Math.floor(min / 60)} 小时前`;
  }

  async function copyPrompt() {
    try {
      await navigator.clipboard.writeText(agentPrompt);
    } catch {
      // Clipboard API needs a secure context; fall back to a hidden textarea.
      const ta = document.createElement("textarea");
      ta.value = agentPrompt;
      document.body.append(ta);
      ta.select();
      document.execCommand("copy");
      ta.remove();
    }
    copied = true;
    setTimeout(() => (copied = false), 2000);
  }
  const popComment = $derived(
    pop?.kind === "view" ? (comments.find((c) => c.id === (pop as { commentId: string }).commentId) ?? null) : null,
  );

  // ----- batch selection -----

  const canResolve = (c: Comment) => c.status === "verify" || c.status === "clarify" || c.status === "orphaned";
  const canDelete = (c: Comment) => c.status === "draft";
  const pickableComment = (c: Comment) => canResolve(c) || canDelete(c);
  const pickableShown = $derived(shown.filter(pickableComment));
  const pickedComments = $derived(comments.filter((c) => picked.has(c.id)));
  const toResolve = $derived(pickedComments.filter(canResolve));
  const toDelete = $derived(pickedComments.filter(canDelete));
  const allPicked = $derived(pickableShown.length > 0 && pickableShown.every((c) => picked.has(c.id)));

  $effect(() => {
    if (allBox) allBox.indeterminate = picked.size > 0 && !allPicked;
  });

  // Picks must not outlive a status change that makes the comment ineligible.
  $effect(() => {
    const valid = new Set(comments.filter(pickableComment).map((c) => c.id));
    for (const pid of [...picked]) if (!valid.has(pid)) picked.delete(pid);
  });

  function setFilter(f: Filter) {
    filter = f;
    picked.clear();
  }

  function toggleAll() {
    if (allPicked) picked.clear();
    else for (const c of pickableShown) picked.add(c.id);
  }

  async function runBatch(list: Comment[], fn: (c: Comment) => Promise<unknown>) {
    batchBusy = true;
    try {
      for (const c of list) await fn(c);
    } catch (e) {
      alert((e as Error).message);
    } finally {
      picked.clear();
      batchBusy = false;
      await load();
    }
  }

  function batchResolve() {
    const list = toResolve;
    if (!list.length) return;
    const clarify = list.filter((c) => c.status === "clarify").length;
    if (clarify && !confirm(`其中 ${clarify} 条是 AI 提出的疑问，还没有回复。仍然全部解决？`)) return;
    runBatch(list, (c) => post(`/api/comments/${c.id}/resolve`));
  }

  function batchDelete() {
    const list = toDelete;
    if (!list.length || !confirm(`删除 ${list.length} 条草稿评论？`)) return;
    runBatch(list, (c) => api("DELETE", `/api/comments/${c.id}`));
  }

  // ----- loading -----

  async function load() {
    try {
      const r = await get<{ report: ReportInfo; version: Version }>(`/api/reports/${id}`);
      const cs = await get<Comment[]>(`/api/reports/${id}/comments`);
      report = r.report;
      version = r.version;
      comments = cs;
      error = "";
    } catch (e) {
      error = (e as Error).message;
    }
  }

  async function refreshRound() {
    const before = report?.active_round?.status;
    const r = await get<{ report: ReportInfo; version: Version }>(`/api/reports/${id}`).catch(() => null);
    if (!r) return;
    if (r.report.active_round?.status !== before || r.version.id !== version?.id) await load();
  }

  onMount(() => {
    load();
    poll = setInterval(() => {
      now = Date.now();
      const s = report?.active_round?.status;
      if (s === "submitted" || s === "processing") refreshRound();
    }, 4000);
  });
  onDestroy(() => {
    clearInterval(poll);
    if ("highlights" in CSS) for (const h of HIGHLIGHTS) CSS.highlights.delete(h);
  });

  // ----- modes -----

  function setMode(m: Mode) {
    if (m === mode) return;
    if (!closePopover()) return;
    mode = m;
    localStorage.setItem(MODE_KEY, m);
    gutter = null;
    activeId = null;
  }

  function editableTarget(t: EventTarget | null) {
    const el = t as HTMLElement | null;
    return !!el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.metaKey || e.ctrlKey || e.altKey || editableTarget(e.target)) return;
    if (e.key === "m" || e.key === "M") {
      e.preventDefault();
      setMode(mode === "read" ? "comment" : "read");
    } else if (e.key === "Escape" && roundOpen) {
      roundOpen = false;
    } else if (e.key === "Escape" && pop) {
      closePopover();
    }
  }

  // ----- popover -----

  /** Returns false when the user chose to keep an unsaved comment. */
  function closePopover(): boolean {
    if (pop?.kind === "compose" && composerDirty && !confirm("放弃这条还没保存的评论？")) return false;
    pop = null;
    composerDirty = false;
    return true;
  }

  function openPopover(p: Popover): boolean {
    if (!closePopover()) return false;
    pop = p;
    return true;
  }

  $effect(() => {
    const p = pop;
    const el = popEl;
    if (!p || !el) return;
    el.style.visibility = "hidden";
    computePosition(p.ref, el, {
      placement: "bottom-start",
      strategy: "absolute",
      middleware: [offset(8), flip({ padding: 8 }), shift({ padding: 8 })],
    }).then(({ x, y }) => {
      el.style.left = `${x}px`;
      el.style.top = `${y}px`;
      el.style.visibility = "visible";
      // A hidden element cannot take focus, so the composer's own autofocus is a no-op here.
      if (p.kind === "compose") el.querySelector("textarea")?.focus({ preventScroll: true });
    });
  });

  // The comment that a view popover showed may be deleted by its own actions.
  $effect(() => {
    if (pop?.kind === "view" && !popComment) pop = null;
  });

  function onDocMouseDown(e: MouseEvent) {
    if (roundOpen && roundEl && !roundEl.contains(e.target as Node)) roundOpen = false;
    if (!pop || !popEl || popEl.contains(e.target as Node)) return;
    // An unsaved comment survives stray clicks; starting a new one asks first.
    if (pop.kind === "compose" && composerDirty) return;
    pop = null;
    composerDirty = false;
  }

  // ----- highlights -----

  function current(c: Comment) {
    return version && c.anchor_version_id === version.id && c.anchor_state !== "orphaned";
  }

  $effect(() => {
    // Track dependencies explicitly; the article HTML must be in the DOM first.
    const cs = comments;
    const act = activeId;
    const el = articleEl;
    const on = commenting;
    const pendingRange = pop?.kind === "compose" ? pop.range : null;
    void version?.html;
    if (!el || !("highlights" in CSS)) return;
    tick().then(() => {
      ranges = new Map();
      const normal: Range[] = [];
      const draft: Range[] = [];
      const active: Range[] = [];
      if (on) {
        for (const c of cs) {
          if (c.status === "resolved" || !current(c) || c.anchor.type !== "text") continue;
          const r = anchorToRange(el, c.anchor);
          if (!r) continue;
          ranges.set(c.id, r);
          (c.id === act ? active : c.status === "draft" ? draft : normal).push(r);
        }
      }
      CSS.highlights.set("galley-comment", new Highlight(...normal));
      CSS.highlights.set("galley-draft", new Highlight(...draft));
      CSS.highlights.set("galley-active", new Highlight(...active));
      CSS.highlights.set("galley-pending", new Highlight(...(pendingRange ? [pendingRange] : [])));
    });
  });

  // Block and section comments are marked with generated CSS, so the report DOM stays untouched.
  const markerCss = $derived.by(() => {
    if (!commenting) return "";
    const rules: string[] = [];
    for (const c of comments) {
      if (c.status === "resolved" || !current(c)) continue;
      const on = c.id === activeId;
      if (c.anchor.type === "block" || c.anchor.type === "cell") {
        const sel =
          c.anchor.type === "cell"
            ? `[data-block="${c.anchor.block_id}"] [data-cell="${c.anchor.row},${c.anchor.col}"]`
            : `[data-block="${c.anchor.block_id}"]`;
        rules.push(`.paper .report ${sel}{background-color:rgba(192,57,43,${on ? 0.16 : 0.06})}`);
      } else if (c.anchor.type === "section") {
        rules.push(
          `.paper .report [data-block="${c.anchor.section_id}"]::after{content:" ●";color:#c0392b;font-size:.55em;vertical-align:middle${on ? ";background:rgba(192,57,43,.2)" : ""}}`,
        );
      }
    }
    if (pop?.kind === "compose") {
      const a = pop.anchor;
      const target = a.type === "block" ? a.block_id : a.type === "section" ? a.section_id : null;
      if (target) rules.push(`.paper .report [data-block="${target}"]{outline:2px solid rgba(192,57,43,.35);outline-offset:4px}`);
    }
    return `<style>${rules.join("\n")}</style>`;
  });

  // ----- composing -----

  /** A reference just below the line under the pointer, so the box opens where the mouse is. */
  function pointerRef(range: Range, x: number, y: number): Ref {
    const rects = Array.from(range.getClientRects()).filter((r) => r.width > 0);
    const line =
      rects.find((r) => y >= r.top && y <= r.bottom) ?? rects[rects.length - 1] ?? range.getBoundingClientRect();
    const left = Math.min(Math.max(x, line.left), line.right);
    const sx = window.scrollX;
    const sy = window.scrollY;
    return {
      getBoundingClientRect: () =>
        new DOMRect(left - (window.scrollX - sx), line.top - (window.scrollY - sy), 0, line.height),
    };
  }

  function onArticleMouseUp(e: MouseEvent) {
    if (!commenting) return;
    const { clientX, clientY } = e;
    setTimeout(() => {
      if (!articleEl) return;
      const sel = getSelection();
      const a = selectionToAnchor(sel, articleEl);
      if (!a || !sel) return;
      const range = sel.getRangeAt(0).cloneRange();
      const ok = openPopover({
        kind: "compose",
        anchor: a,
        label: null,
        ref: pointerRef(range, clientX, clientY),
        range,
      });
      if (ok) activeId = null;
      else sel.removeAllRanges();
    }, 0);
  }

  function startBlock(blockId: string, heading: boolean) {
    const b = version?.doc.blocks.find((x) => x.id === blockId);
    const el = articleEl?.querySelector<HTMLElement>(`[data-block="${blockId}"]`);
    if (!el) return;
    const anchor: Anchor = heading ? { type: "section", section_id: blockId } : { type: "block", block_id: blockId };
    const ok = openPopover({
      kind: "compose",
      anchor,
      label: heading ? `评论整章「${truncate(b?.text ?? "", 24)}」` : "评论整段",
      ref: el,
      range: null,
    });
    if (ok) activeId = null;
  }

  function startSection(sectionId: string | null) {
    if (mode !== "comment") setMode("comment");
    if (sectionId !== null) {
      articleEl?.querySelector(`[data-block="${sectionId}"]`)?.scrollIntoView({ block: "center" });
      startBlock(sectionId, true);
      return;
    }
    if (!titleEl) return;
    titleEl.scrollIntoView({ block: "center" });
    if (openPopover({ kind: "compose", anchor: { type: "document" }, label: "评论整篇报告", ref: titleEl, range: null }))
      activeId = null;
  }

  async function saveComment(body: string) {
    if (pop?.kind !== "compose") return;
    const c = await post<Comment>(`/api/reports/${id}/comments`, { body, anchor: pop.anchor });
    pop = null;
    composerDirty = false;
    getSelection()?.removeAllRanges();
    await load();
    activeId = c.id;
    await tick();
    document.getElementById(`card-${c.id}`)?.scrollIntoView({ block: "nearest" });
  }

  // ----- gutter (block / section comments) -----

  function onMouseMove(e: MouseEvent) {
    if (!commenting || !paperEl || !articleEl) return;
    const t = e.target as Element;
    if (t.closest(".gutter-add") || t.closest(".pop")) return;
    const block = t.closest?.("[data-block]") as HTMLElement | null;
    if (!block || !articleEl.contains(block)) return;
    const top = block.getBoundingClientRect().top - paperEl.getBoundingClientRect().top;
    gutter = { top, blockId: block.dataset.block!, heading: /^H[1-6]$/.test(block.tagName) };
  }

  // ----- clicking highlighted text opens its comment in place -----

  function onArticleClick(e: MouseEvent) {
    if (!commenting) return;
    const sel = getSelection();
    if (sel && !sel.isCollapsed) return;
    const doc = document as Document & {
      caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null;
      caretRangeFromPoint?: (x: number, y: number) => Range | null;
    };
    let node: Node | null = null;
    let off = 0;
    const pos = doc.caretPositionFromPoint?.(e.clientX, e.clientY);
    if (pos) {
      node = pos.offsetNode;
      off = pos.offset;
    } else {
      const r = doc.caretRangeFromPoint?.(e.clientX, e.clientY);
      if (r) {
        node = r.startContainer;
        off = r.startOffset;
      }
    }
    if (node) {
      for (const [cid, r] of ranges) {
        try {
          if (r.isPointInRange(node, off)) {
            showInline(cid, pointerRef(r, e.clientX, e.clientY));
            return;
          }
        } catch {
          /* node outside the range's document */
        }
      }
    }
    const block = (e.target as Element).closest("[data-block]") as HTMLElement | null;
    if (block) {
      const hit = comments.find(
        (c) =>
          c.status !== "resolved" &&
          current(c) &&
          ((c.anchor.type === "block" && c.anchor.block_id === block.dataset.block) ||
            (c.anchor.type === "section" && c.anchor.section_id === block.dataset.block)),
      );
      if (hit) showInline(hit.id, block);
    }
  }

  async function showInline(cid: string, ref: Ref) {
    if (!openPopover({ kind: "view", commentId: cid, ref })) return;
    activeId = cid;
    await tick();
    document.getElementById(`card-${cid}`)?.scrollIntoView({ block: "nearest" });
  }

  /** Sidebar click: bring the commented text into view and mark it. */
  function focusComment(cid: string) {
    if (!closePopover()) return;
    activeId = cid;
    const c = comments.find((x) => x.id === cid);
    if (!c || !current(c) || !articleEl) return;
    if (c.anchor.type === "document") {
      window.scrollTo({ top: 0 });
      return;
    }
    const target = ranges.get(cid)?.startContainer.parentElement ?? scopeFor(articleEl, c.anchor);
    target?.scrollIntoView({ block: "center" });
  }

  function jump(sectionId: string | null) {
    if (!sectionId) {
      window.scrollTo({ top: 0 });
      return;
    }
    articleEl?.querySelector(`[data-block="${sectionId}"]`)?.scrollIntoView({ block: "start" });
  }

  async function submitRound() {
    const open = submittable - drafts;
    const msg = `提交第 ${(report?.round_count ?? 0) + 1} 轮：${drafts} 条新评论${open ? `，${open} 条重新打开/待处理` : ""}。提交后 AI 才能看到，草稿将不可再编辑。`;
    if (!confirm(msg)) return;
    submitting = true;
    try {
      await post(`/api/reports/${id}/rounds`);
      await load();
    } catch (e) {
      alert((e as Error).message);
    } finally {
      submitting = false;
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />
<svelte:document onmousedown={onDocMouseDown} />

<TopBar {report} active="workbench">
  {#if report}
    {#if round}
      <div class="round-status" bind:this={roundEl}>
        {#if round.status === "verifying"}
          <a class="round-chip verifying" href="/app/r/{id}/verify">第 {round.seq} 轮待验证 →</a>
        {:else}
          <button
            class="round-chip"
            aria-expanded={roundOpen}
            title={round.status === "processing" && round.claimed_at ? `已认领 ${ago(round.claimed_at)}` : ""}
            onclick={() => (roundOpen = !roundOpen)}
          >
            <span class="dot" class:busy={round.status === "processing"}></span>
            第 {round.seq} 轮 · {round.status === "submitted" ? "等待 AI" : "AI 处理中"}
          </button>
          {#if roundOpen}
            <div class="round-pop" role="dialog" aria-label="本轮状态">
              <div class="small">
                {round.comment_count} 条评论 · 提交于 {ago(round.submitted_at)}{#if round.claimed_at} · 已认领 {ago(round.claimed_at)}{/if}
              </div>
              {#if round.status === "submitted"}
                <div class="muted small">Galley 不会自己调用 AI。把这句话发给已接入 Galley 的 AI 工具：</div>
                <code class="prompt">{agentPrompt}</code>
                <div class="row">
                  <a class="small" href="/app#agent">接入方法</a>
                  <span class="spacer"></span>
                  <button class="primary" onclick={copyPrompt}>{copied ? "已复制" : "复制"}</button>
                </div>
              {:else}
                <div class="muted small">AI 正在修改，完成后这里会变成「待验证」。期间可以继续写下一轮的草稿评论。</div>
              {/if}
            </div>
          {/if}
        {/if}
      </div>
    {/if}
    <div class="seg" role="group" aria-label="模式">
      <button class:on={mode === "read"} aria-pressed={mode === "read"} title="阅读模式（M 切换）" onclick={() => setMode("read")}>阅读</button>
      <button class:on={mode === "comment"} aria-pressed={mode === "comment"} title="评论模式（M 切换）" onclick={() => setMode("comment")}>评论</button>
    </div>
    <button class="primary" disabled={submittable === 0 || roundBusy || submitting} onclick={submitRound}
      title={roundBusy ? "当前轮次还未结束" : ""}>
      提交本轮{#if submittable}（{submittable}）{/if}
    </button>
  {/if}
</TopBar>

{#if error}<div class="banner attention">{error}</div>{/if}

{#if version && report}
  {@html markerCss}
  <div class="workbench" class:reading={!commenting}>
    <Outline blocks={version.doc.blocks} {comments} editable={commenting} onjump={jump} onadd={startSection} />

    <div class="paper" bind:this={paperEl} onmousemove={onMouseMove} onmouseleave={() => (gutter = null)} role="presentation">
      <div class="report"><h1 class="report-title" bind:this={titleEl}>{version.doc.title}</h1></div>
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <article class="report" bind:this={articleEl} onmouseup={onArticleMouseUp} onclick={onArticleClick}>
        {@html version.html}
      </article>
      {#if commenting && gutter}
        <button
          class="gutter-add"
          style="top: {gutter.top + 4}px; left: 18px"
          title={gutter.heading ? "评论本章节" : "评论整段"}
          onclick={() => gutter && startBlock(gutter.blockId, gutter.heading)}>＋</button>
      {/if}
      {#if pop}
        <div class="pop" bind:this={popEl} role="dialog" aria-label={pop.kind === "compose" ? "写评论" : "评论"}>
          {#if pop.kind === "compose"}
            {#key pop}
              <Composer
                targetLabel={pop.label}
                bind:dirty={composerDirty}
                onsave={saveComment}
                oncancel={() => closePopover()}
              />
            {/key}
          {:else if popComment}
            <CommentCard
              comment={popComment}
              inline
              sectionTitle={popComment.section_id ? titles.get(popComment.section_id) : null}
              onselect={() => {}}
              onchanged={load}
            />
          {/if}
        </div>
      {/if}
    </div>

    {#if commenting}
      <aside class="sidebar" aria-label="评论">
        <div class="sidebar-head">
          <div class="filters" role="tablist" aria-label="按状态筛选">
            <button role="tab" aria-selected={filter === "unresolved"} class:on={filter === "unresolved"} onclick={() => setFilter("unresolved")}>
              未解决 <span class="n">{unresolvedCount}</span>
            </button>
            {#each STATUS_ORDER as s (s)}
              {#if statusCounts.get(s) || filter === s}
                <button role="tab" aria-selected={filter === s} class:on={filter === s} onclick={() => setFilter(s)}>
                  {STATUS_LABEL[s]} <span class="n">{statusCounts.get(s) ?? 0}</span>
                </button>
              {/if}
            {/each}
            <button role="tab" aria-selected={filter === "resolved"} class:on={filter === "resolved"} onclick={() => setFilter("resolved")}>
              已解决 <span class="n">{statusCounts.get("resolved") ?? 0}</span>
            </button>
            <button role="tab" aria-selected={filter === "all"} class:on={filter === "all"} onclick={() => setFilter("all")}>
              全部 <span class="n">{comments.length}</span>
            </button>
          </div>
        </div>
        <div class="batchbar">
          {#if pickableShown.length}
            <label class="all">
              <input type="checkbox" bind:this={allBox} checked={allPicked} onchange={toggleAll} />
              {picked.size ? `已选 ${picked.size}` : "全选"}
            </label>
            {#if toResolve.length}
              <button class="primary" disabled={batchBusy} onclick={batchResolve}>解决（{toResolve.length}）</button>
            {/if}
            {#if toDelete.length}
              <button disabled={batchBusy} onclick={batchDelete}>删除草稿（{toDelete.length}）</button>
            {/if}
          {/if}
          <span class="spacer"></span>
          <button class="quiet" onclick={() => startSection(null)}>整篇评论</button>
        </div>
        <div class="sidebar-body">
          {#each shown as c (c.id)}
            <CommentCard
              comment={c}
              active={c.id === activeId}
              sectionTitle={c.section_id ? titles.get(c.section_id) : null}
              pickable={pickableComment(c)}
              picked={picked.has(c.id)}
              onpick={(on) => (on ? picked.add(c.id) : picked.delete(c.id))}
              onselect={() => focusComment(c.id)}
              onchanged={load}
            />
          {:else}
            <div class="empty">
              {#if comments.length}
                这个筛选下没有评论。
              {:else}
                选中正文文字，评论框会直接出现在鼠标下方；鼠标移到段落左侧点「＋」评论整段；在左侧大纲评论整章。
              {/if}
            </div>
          {/each}
        </div>
      </aside>
    {:else}
      <aside class="rail" aria-label="评论概况">
        <button class="rail-btn" title="切换到评论模式（M）" onclick={() => setMode("comment")}>
          <span class="rail-item"><b>{unresolvedCount}</b>未解决</span>
          {#each STATUS_ORDER as s (s)}
            {#if statusCounts.get(s)}
              <span class="rail-item s-{s}"><b>{statusCounts.get(s)}</b>{STATUS_LABEL[s]}</span>
            {/if}
          {/each}
          <span class="rail-go">展开</span>
        </button>
      </aside>
    {/if}
  </div>
{:else if !error}
  <div class="empty">加载中…</div>
{/if}
