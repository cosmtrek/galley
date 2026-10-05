<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { computePosition, flip, offset, shift } from "@floating-ui/dom";
  import { api, get, post } from "../lib/api";
  import { ask, confirmState } from "../lib/confirm.svelte";
  import { anchorToRange, scopeFor, selectionToAnchor } from "../lib/anchor";
  import { commentPosition, fmtAgo, STATUS_LABEL, truncate } from "../lib/format";
  import { diffCtx, partner, withoutPartners, type DiffCtx } from "../lib/diffctx";
  import type { Anchor, Comment, CommentStatus, ExtraChange, ReportInfo, Review, ReviewItem, Round, Version } from "../lib/types";
  import ChangeView from "../components/ChangeView.svelte";
  import PromptBox from "../components/PromptBox.svelte";
  import TopBar from "../components/TopBar.svelte";
  import Outline from "../components/Outline.svelte";
  import CommentCard from "../components/CommentCard.svelte";
  import Composer from "../components/Composer.svelte";

  let { id }: { id: string } = $props();

  type Mode = "read" | "comment";
  type GroupKey = "mine" | "extra" | "agent" | "draft" | "resolved";
  type Group = { key: GroupKey; label: string; items: Comment[] };
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
  let ranges = new Map<string, Range>();
  let poll: ReturnType<typeof setInterval> | undefined;

  const archived = $derived(!!report?.archived_at);
  const commenting = $derived(mode === "comment" && !archived);
  const titles = $derived(
    new Map((version?.doc.blocks ?? []).filter((b) => b.kind === "heading").map((b) => [b.id, b.text])),
  );
  const blocks = $derived(new Map((version?.doc.blocks ?? []).map((b) => [b.id, b])));
  const anchorBlock = (c: Comment) => (c.anchor.type === "block" ? blocks.get(c.anchor.block_id) : null);
  const statusCounts = $derived.by(() => {
    const m = new Map<CommentStatus, number>();
    for (const c of comments) m.set(c.status, (m.get(c.status) ?? 0) + 1);
    return m;
  });
  const unresolvedCount = $derived(comments.length - (statusCounts.get("resolved") ?? 0));
  const blockIndex = $derived(new Map((version?.doc.blocks ?? []).map((b, i) => [b.id, i])));
  const shown = $derived(
    comments
      .slice()
      .sort((a, b) => {
        const [pa, sa] = commentPosition(a.anchor, blockIndex);
        const [pb, sb] = commentPosition(b.anchor, blockIndex);
        if (pa !== pb) return pa < pb ? -1 : 1;
        return sa - sb || a.created_at - b.created_at;
      }),
  );
  const groupOf = (c: Comment): GroupKey =>
    c.status === "verify" || c.status === "clarify" || c.status === "orphaned"
      ? "mine"
      : c.status === "open"
        ? "agent"
        : c.status === "draft"
          ? "draft"
          : "resolved";
  const GROUPS: { key: GroupKey; label: string }[] = [
    { key: "mine", label: "需要我处理" },
    { key: "extra", label: "评论之外的改动" },
    { key: "agent", label: "等 AI 处理" },
    { key: "draft", label: "草稿" },
    { key: "resolved", label: "已解决" },
  ];
  const groups = $derived<Group[]>(
    GROUPS.map((g) => ({ ...g, items: shown.filter((c) => groupOf(c) === g.key) })).filter(
      (g) => g.items.length || (g.key === "extra" && extraPending.length),
    ),
  );
  let resolvedOpen = $state(false);
  const submittable = $derived(comments.filter((c) => c.status === "draft" || c.status === "open").length);
  const round = $derived<Round | null>(report?.active_round ?? null);

  // ----- round under verification: what changed -----

  let review = $state<Review | null>(null);
  let ctx = $state<DiffCtx | null>(null);
  // A string key, so reloading the report does not refetch an unchanged round.
  const verifyKey = $derived(
    round?.status === "verifying" ? [round.id, round.base_version_id, round.result_version_id ?? ""].join("|") : "",
  );
  $effect(() => {
    const key = verifyKey;
    review = null;
    ctx = null;
    if (!key) return;
    const [rid, base, result] = key.split("|");
    let stale = false;
    Promise.all([
      get<Review>(`/api/rounds/${rid}/review`),
      result ? diffCtx(base, result).catch(() => null) : Promise.resolve(null),
    ])
      .then(([r, c]) => {
        if (stale) return;
        review = r;
        ctx = c;
      })
      .catch(() => {});
    return () => (stale = true);
  });
  const reviewOf = $derived(new Map<string, ReviewItem>((review?.items ?? []).map((it) => [it.comment.id, it])));
  const verifyCount = $derived(statusCounts.get("verify") ?? 0);

  const extraAll = $derived<ExtraChange[]>(round?.status === "verifying" ? round.extra_changes : []);
  const extraPending = $derived(withoutPartners(ctx, extraAll).filter((e) => !e.confirmed));
  const sectionTitle = (sid: string) => review?.section_titles[sid] ?? titles.get(sid) ?? "开头";

  // A replacement is displayed as one item, so acting on it covers both halves.
  function pairIds(ch: ExtraChange) {
    const p = partner(ctx, ch);
    return [ch.block_id, ...(p && extraAll.some((e) => e.block_id === p.block_id && !e.confirmed) ? [p.block_id] : [])];
  }
  const revertBody = (ch: ExtraChange) =>
    partner(ctx, ch) || ch.op === "modified" || ch.op === "moved"
      ? "请恢复为修改前的内容。"
      : ch.op === "added"
        ? "这段是新增的，我不需要，请删掉。"
        : "这段被删掉了，请恢复。";

  async function runExtra(fn: (rid: string) => Promise<unknown>) {
    if (!round) return;
    batchBusy = true;
    actionError = "";
    try {
      await fn(round.id);
    } catch (e) {
      actionError = (e as Error).message;
    } finally {
      batchBusy = false;
      await load();
    }
  }
  const confirmExtra = (ch: ExtraChange) =>
    runExtra(async (rid) => {
      for (const b of pairIds(ch)) await post(`/api/rounds/${rid}/extra/confirm`, { block_id: b });
    });
  async function confirmAllExtra() {
    const n = extraPending.length;
    if (await ask({ title: `确认全部 ${n} 处评论之外的改动？`, message: "确认后这些改动会保留在报告里。", confirmLabel: "全部确认" }))
      runExtra((rid) => post(`/api/rounds/${rid}/extra/confirm`, {}));
  }
  // The draft comment it creates shows up under 草稿, where it can still be edited before submitting.
  const revertExtra = (ch: ExtraChange) =>
    runExtra((rid) => post(`/api/rounds/${rid}/extra/revert`, { block_ids: pairIds(ch), body: revertBody(ch) }));

  function focusExtra(ch: ExtraChange) {
    const target = ch.op === "deleted" ? (partner(ctx, ch)?.block_id ?? ch.after) : ch.block_id;
    if (target) articleEl?.querySelector(`[data-block="${target}"]`)?.scrollIntoView({ block: "center" });
  }
  let now = $state(Date.now());
  let actionError = $state("");
  // Reading `now` makes the relative times refresh on the poll tick.
  const ago = (ms: number | null) => (void now, ms ? fmtAgo(ms) : "");

  const popComment = $derived(
    pop?.kind === "view" ? (comments.find((c) => c.id === (pop as { commentId: string }).commentId) ?? null) : null,
  );

  // ----- batch selection -----

  const canResolve = (c: Comment) => c.status === "verify" || c.status === "clarify" || c.status === "orphaned";
  const canDelete = (c: Comment) => c.status === "draft";
  const pickableComment = (c: Comment) => canResolve(c) || canDelete(c);
  const pickedComments = $derived(comments.filter((c) => picked.has(c.id)));
  const toResolve = $derived(pickedComments.filter(canResolve));
  const toDelete = $derived(pickedComments.filter(canDelete));

  // Picks must not outlive a status change that makes the comment ineligible.
  $effect(() => {
    const valid = new Set(comments.filter(pickableComment).map((c) => c.id));
    for (const pid of [...picked]) if (!valid.has(pid)) picked.delete(pid);
  });

  async function runBatch(list: Comment[], fn: (c: Comment) => Promise<unknown>) {
    batchBusy = true;
    actionError = "";
    try {
      for (const c of list) await fn(c);
    } catch (e) {
      actionError = (e as Error).message;
    } finally {
      picked.clear();
      batchBusy = false;
      await load();
    }
  }

  async function resolveMany(list: Comment[]) {
    if (!list.length) return;
    const clarify = list.filter((c) => c.status === "clarify").length;
    const ok = await ask({
      title: `解决 ${list.length} 条评论？`,
      message: clarify ? `其中 ${clarify} 条是 AI 提出的疑问，还没有回复。` : undefined,
      confirmLabel: "解决",
    });
    if (ok) runBatch(list, (c) => post(`/api/comments/${c.id}/resolve`));
  }

  async function batchDelete() {
    const list = toDelete;
    if (!list.length) return;
    if (await ask({ title: `删除 ${list.length} 条草稿评论？`, message: "删除后无法恢复。", confirmLabel: "删除", danger: true }))
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

  async function setMode(m: Mode) {
    if (m === mode) return;
    if (!(await closePopover())) return;
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
    // While the confirm dialog is open, Escape belongs to it.
    if (e.metaKey || e.ctrlKey || e.altKey || confirmState.current || editableTarget(e.target)) return;
    if (e.key === "m" || e.key === "M") {
      e.preventDefault();
      setMode(mode === "read" ? "comment" : "read");
    } else if (e.key === "Escape" && pop) {
      e.preventDefault();
      closePopover();
    }
  }

  // ----- popover -----

  /** Resolves false when the user chose to keep an unsaved comment. */
  async function closePopover(): Promise<boolean> {
    if (
      pop?.kind === "compose" &&
      composerDirty &&
      !(await ask({ title: "放弃这条还没保存的评论？", confirmLabel: "放弃", danger: true }))
    )
      return false;
    pop = null;
    composerDirty = false;
    return true;
  }

  async function openPopover(p: Popover): Promise<boolean> {
    if (!(await closePopover())) return false;
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
        rules.push(`.paper .report ${sel}{background-color:color-mix(in srgb,var(--accent) ${on ? 16 : 6}%,transparent)}`);
      } else if (c.anchor.type === "section") {
        rules.push(
          `.paper .report [data-block="${c.anchor.section_id}"]::after{content:" ●";color:var(--accent);font-size:.55em;vertical-align:middle${on ? ";background:color-mix(in srgb,var(--accent) 20%,transparent)" : ""}}`,
        );
      }
    }
    if (pop?.kind === "compose") {
      const a = pop.anchor;
      const target = a.type === "block" ? a.block_id : a.type === "section" ? a.section_id : null;
      if (target) rules.push(`.paper .report [data-block="${target}"]{outline:2px solid color-mix(in srgb,var(--accent) 35%,transparent);outline-offset:4px}`);
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
    setTimeout(async () => {
      if (!articleEl) return;
      const sel = getSelection();
      const a = selectionToAnchor(sel, articleEl);
      if (!a || !sel) return;
      const range = sel.getRangeAt(0).cloneRange();
      const ok = await openPopover({
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

  async function startBlock(blockId: string, heading: boolean) {
    const b = version?.doc.blocks.find((x) => x.id === blockId);
    const el = articleEl?.querySelector<HTMLElement>(`[data-block="${blockId}"]`);
    if (!el) return;
    const anchor: Anchor = heading ? { type: "section", section_id: blockId } : { type: "block", block_id: blockId };
    const ok = await openPopover({
      kind: "compose",
      anchor,
      label: heading ? `评论整章「${truncate(b?.text ?? "", 24)}」` : "评论整段",
      ref: el,
      range: null,
    });
    if (ok) activeId = null;
  }

  async function startSection(sectionId: string | null) {
    if (mode !== "comment") await setMode("comment");
    if (sectionId !== null) {
      articleEl?.querySelector(`[data-block="${sectionId}"]`)?.scrollIntoView({ block: "center" });
      startBlock(sectionId, true);
      return;
    }
    if (!titleEl) return;
    titleEl.scrollIntoView({ block: "center" });
    if (await openPopover({ kind: "compose", anchor: { type: "document" }, label: "整篇评论", ref: titleEl, range: null }))
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
    if (!(await openPopover({ kind: "view", commentId: cid, ref }))) return;
    activeId = cid;
    await tick();
    document.getElementById(`card-${cid}`)?.scrollIntoView({ block: "nearest" });
  }

  /** Sidebar click: bring the commented text into view and mark it. */
  async function focusComment(cid: string) {
    if (!(await closePopover())) return;
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
    submitting = true;
    actionError = "";
    try {
      await post(`/api/reports/${id}/rounds`);
      await load();
    } catch (e) {
      actionError = (e as Error).message;
    } finally {
      submitting = false;
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />
<svelte:document onmousedown={onDocMouseDown} />

<TopBar {report} active="workbench" />

{#if report && archived}
  <div class="banner">已归档，只读。<a href="/app/r/{id}/publish">到发布页恢复 →</a></div>
{/if}
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
          style="top: {gutter.top + 4}px"
          title={gutter.heading ? "评论整章" : "评论整段"}
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
              block={anchorBlock(popComment)}
              review={reviewOf.get(popComment.id)}
              {ctx}
              reportId={id}
              onselect={() => {}}
              onchanged={load}
            />
          {/if}
        </div>
      {/if}
    </div>

    {#if commenting}
      <aside class="sidebar" class:picking={picked.size > 0} aria-label="评论">
        <div class="sidebar-head">
          <span class="head-title">评论</span>
          <span class="muted">{comments.length}</span>
          <span class="spacer"></span>
          <button class="quiet sm" onclick={() => startSection(null)}>＋ 整篇评论</button>
          <button class="quiet sm" title="收起评论栏，进入阅读模式（M）" onclick={() => setMode("read")}>收起</button>
        </div>
        {#if picked.size > 0}
          <div class="batchbar">
            <span class="muted small">已选 {picked.size} 条</span>
            <span class="spacer"></span>
            <button class="quiet sm" onclick={() => picked.clear()}>取消</button>
            {#if toDelete.length}
              <button class="danger sm" disabled={batchBusy} onclick={batchDelete}>删除草稿</button>
            {/if}
            {#if toResolve.length}
              <button class="primary sm" disabled={batchBusy} onclick={() => resolveMany(toResolve)}>解决</button>
            {/if}
          </div>
        {/if}
        <div class="sidebar-body">
          {#snippet card(c: Comment)}
            <CommentCard
              comment={c}
              active={c.id === activeId}
              sectionTitle={c.section_id ? titles.get(c.section_id) : null}
              block={anchorBlock(c)}
              review={reviewOf.get(c.id)}
              {ctx}
              reportId={id}
              pickable={pickableComment(c)}
              picked={picked.has(c.id)}
              onpick={(on) => (on ? picked.add(c.id) : picked.delete(c.id))}
              onselect={() => focusComment(c.id)}
              onchanged={load}
            />
          {/snippet}
          {#each groups as g (g.key)}
              {@const verifyList = g.key === "mine" ? g.items.filter((c) => c.status === "verify") : []}
              {#if g.key === "resolved"}
                <button class="group-head toggle" aria-expanded={resolvedOpen} onclick={() => (resolvedOpen = !resolvedOpen)}>
                  <span>{resolvedOpen ? "▼" : "▶"} {g.label} · {g.items.length}</span>
                </button>
              {:else if g.key === "extra"}
                <div class="group-head">
                  <span>{g.label} · {extraPending.length}</span>
                  <span class="spacer"></span>
                  <button class="link" disabled={batchBusy} onclick={confirmAllExtra}>全部确认</button>
                </div>
                {#each extraPending as ch (ch.op + ch.block_id)}
                  <div
                    class="card s-extra"
                    role="button"
                    tabindex="0"
                    onclick={() => focusExtra(ch)}
                    onkeydown={(e) => e.key === "Enter" && e.target === e.currentTarget && focusExtra(ch)}
                  >
                    <div class="card-loc">{sectionTitle(ch.section_id)}</div>
                    <div class="card-change"><ChangeView change={ch} {ctx} showOp={ch.op !== "modified"} /></div>
                    <div class="card-foot mine" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="presentation">
                      <span class="card-status">待确认</span>
                      <span class="spacer"></span>
                      <button class="quiet sm" disabled={batchBusy} title="生成一条草稿评论，随下一轮交给 AI 改回去" onclick={() => revertExtra(ch)}>改回去</button>
                      <button class="primary sm" disabled={batchBusy} onclick={() => confirmExtra(ch)}>确认</button>
                    </div>
                  </div>
                {/each}
              {:else}
                <div class="group-head">
                  <span>{g.label} · {g.items.length}</span>
                  <span class="spacer"></span>
                  {#if verifyList.length > 1}
                    <button class="link" disabled={batchBusy} onclick={() => resolveMany(verifyList)}>全部解决</button>
                  {/if}
                </div>
              {/if}
              {#if g.key !== "resolved" || resolvedOpen}
                {#each g.items as c (c.id)}
                  {@render card(c)}
                {/each}
              {/if}
          {:else}
            <div class="empty">
              选中正文文字即可评论；鼠标移到段落左侧点「＋」评论整段；在左侧大纲评论整章。
            </div>
          {/each}
        </div>
        {#if round || submittable > 0 || actionError}
          <div class="sidebar-foot">
            {#if actionError}
              <p class="error small">
                <span class="spacer">{actionError}</span>
                <button class="link" onclick={() => (actionError = "")}>知道了</button>
              </p>
            {/if}
            {#if round}
              {#if round.status === "verifying"}
                <div class="foot-line">
                  <span class="dot ok"></span>
                  第 {round.seq} 轮 · {verifyCount
                    ? `${verifyCount} 条待验证`
                    : extraPending.length
                      ? `还有 ${extraPending.length} 处评论之外的改动待确认`
                      : "处理完毕"}
                </div>
                {#if round.summary}
                  <div class="ai-summary"><span class="muted">AI 摘要：</span>{round.summary}</div>
                {/if}
              {:else}
                <div class="foot-line">
                  <span class="dot {round.status === 'processing' ? 'ok' : 'warn'}"></span>
                  第 {round.seq} 轮 · {round.status === "submitted" ? "等待 AI" : "AI 处理中"}
                  <span class="spacer"></span>
                  <span class="muted small">
                    {round.comment_count} 条评论 · {round.claimed_at ? `${ago(round.claimed_at)}认领` : `${ago(round.submitted_at)}提交`}
                  </span>
                </div>
                {#if round.status === "submitted"}
                  <PromptBox {report} {round} />
                {:else}
                  <div class="muted small">AI 正在修改，期间可以继续写下一轮的草稿评论。</div>
                {/if}
              {/if}
            {:else if submittable > 0}
              <div class="foot-line">
                <span>{submittable} 条评论待提交</span>
                <span class="spacer"></span>
                <button class="primary" disabled={submitting} onclick={submitRound}>提交本轮</button>
              </div>
            {/if}
          </div>
        {/if}
      </aside>
    {:else}
      <aside class="rail" aria-label="评论概况">
        <button class="rail-btn" title="切换到评论模式（M）" disabled={archived} onclick={() => setMode("comment")}>
          <span class="rail-item"><b>{unresolvedCount}</b>未解决</span>
          {#each STATUS_ORDER as s (s)}
            {#if statusCounts.get(s)}
              <span class="rail-item s-{s}"><b>{statusCounts.get(s)}</b>{STATUS_LABEL[s]}</span>
            {/if}
          {/each}
          {#if round}
            <span class="rail-item round"><b>第{round.seq}轮</b><span class:accent={round.status === "verifying"}>{round.status === "verifying" ? "待验证" : round.status === "processing" ? "处理中" : "等待 AI"}</span></span>
          {:else if submittable > 0}
            <span class="rail-item"><b>{submittable}</b>待提交</span>
          {/if}
          {#if !archived}<span class="rail-go">展开</span>{/if}
        </button>
      </aside>
    {/if}
  </div>
{:else if !error}
  <div class="empty">加载中…</div>
{/if}
