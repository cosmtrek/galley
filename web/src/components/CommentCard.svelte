<script lang="ts">
  import { api, post } from "../lib/api";
  import { ask } from "../lib/confirm.svelte";
  import { anchorLabel, blockSummary, fmtTime, STATUS_LABEL, truncate } from "../lib/format";
  import { withoutPartners, type DiffCtx } from "../lib/diffctx";
  import type { Block, Comment, ReviewItem } from "../lib/types";
  import ChangeView from "./ChangeView.svelte";
  import Composer from "./Composer.svelte";
  import Thread from "./Thread.svelte";

  let {
    comment,
    active = false,
    sectionTitle = null,
    block = null,
    review = null,
    ctx = null,
    reportId = "",
    inline = false,
    pickable = false,
    picked = false,
    onpick,
    onselect,
    onchanged,
  }: {
    comment: Comment;
    active?: boolean;
    sectionTitle?: string | null;
    /** Current block of a block/cell anchor, used for a kind-specific label. */
    block?: Block | null;
    /** What the round under verification changed for this comment. */
    review?: ReviewItem | null;
    ctx?: DiffCtx | null;
    reportId?: string;
    inline?: boolean;
    pickable?: boolean;
    picked?: boolean;
    onpick?: (on: boolean) => void;
    onselect: () => void;
    onchanged: () => Promise<void> | void;
  } = $props();

  let editing = $state(false);
  let replying = $state<"" | "message" | "reopen">("");
  let busy = $state(false);
  let error = $state("");

  const quote = $derived(
    comment.anchor.type === "text"
      ? comment.anchor.quote
      : block && comment.anchor.type === "block"
        ? blockSummary(block)
        : comment.original_quote,
  );
  const changedQuote = $derived(
    comment.original_quote && quote && comment.original_quote !== quote ? comment.original_quote : null,
  );

  async function run(fn: () => Promise<unknown>) {
    busy = true;
    error = "";
    try {
      await fn();
      await onchanged();
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  const resolve = () => run(() => post(`/api/comments/${comment.id}/resolve`));
  async function remove() {
    if (await ask({ title: "删除这条草稿评论？", message: "删除后无法恢复。", confirmLabel: "删除", danger: true }))
      run(() => api("DELETE", `/api/comments/${comment.id}`));
  }
  // Errors surface inside the composer, next to the text being sent.
  async function sendReply(body: string) {
    const path = replying === "reopen" ? "reopen" : "messages";
    await post(`/api/comments/${comment.id}/${path}`, { body });
    replying = "";
    await onchanged();
  }

  // Items waiting on the owner keep their actions visible; the rest reveal them on hover.
  const mine = $derived(comment.status === "verify" || comment.status === "clarify" || comment.status === "orphaned");
  const toggle = (kind: "message" | "reopen") => (replying = replying === kind ? "" : kind);

  function stop(e: Event) {
    e.stopPropagation();
  }
</script>

<div
  class="card s-{comment.status}"
  class:active
  class:inline
  id={inline ? undefined : `card-${comment.id}`}
  onclick={onselect}
  onkeydown={(e) => e.key === "Enter" && e.target === e.currentTarget && onselect()}
  role="button"
  tabindex="0"
>
  {#if pickable && !editing}
    <input
      type="checkbox"
      class="pick"
      checked={picked}
      aria-label="选择这条评论"
      onclick={stop}
      onchange={(e) => onpick?.((e.currentTarget as HTMLInputElement).checked)}
    />
  {/if}

  {#if editing}
    <div onclick={stop} onkeydown={stop} role="presentation">
      <Composer
        initialBody={comment.body}
        onsave={async (body) => {
          await api("PATCH", `/api/comments/${comment.id}`, { body });
          editing = false;
          await onchanged();
        }}
        oncancel={() => (editing = false)}
      />
    </div>
  {:else}
    <div class="body" title={comment.id}>{comment.body}</div>
  {/if}

  <div class="card-loc">
    {#if comment.anchor.type === "text"}
      {sectionTitle ? sectionTitle + " · " : ""}「{truncate(comment.anchor.quote, 120)}」
    {:else}
      {anchorLabel(comment.anchor, sectionTitle, block?.kind)}{#if quote} ·「{truncate(quote, 80)}」{/if}
    {/if}
  </div>
  {#if changedQuote && block?.kind !== "table"}<div class="muted small">原文：{truncate(changedQuote, 80)}</div>{/if}
  {#if comment.anchor_state === "orphaned" && comment.status !== "resolved"}
    <div class="warn-note">锚定的内容已不在当前版本中。</div>
  {:else if comment.anchor_state === "fuzzy" && comment.status !== "resolved"}
    <div class="warn-note">原文已改动，位置为近似匹配。</div>
  {/if}

  <Thread messages={comment.messages} />

  {#if review && comment.status === "verify"}
    <div class="card-change">
      {#if review.change}
        <ChangeView change={review.change} {ctx} showOp={false} />
      {:else if review.section_changes.length}
        {#each withoutPartners(ctx, review.section_changes) as ch (ch.op + ch.block_id)}
          <ChangeView change={ch} {ctx} />
        {/each}
      {:else if comment.anchor.type === "document"}
        <div class="muted small">
          整篇评论，改动分布在全文。<a href="/app/r/{reportId}/history">查看全文对比 →</a>
        </div>
      {:else}
        <div class="muted small">这里没有文字改动。</div>
      {/if}
    </div>
  {/if}

  {#if !editing}
    <div class="card-foot" class:mine onclick={stop} onkeydown={stop} role="presentation">
      <span class="card-status">{STATUS_LABEL[comment.status]} · {fmtTime(comment.created_at)}</span>
      <span class="spacer"></span>
      {#if replying}
        <!-- the open composer below carries the actions -->
      {:else if comment.status === "draft"}
        <button class="quiet sm" onclick={() => (editing = true)}>编辑</button>
        <button class="quiet sm" onclick={remove}>删除</button>
      {:else if comment.status === "open"}
        <button class="quiet sm" onclick={() => toggle("message")}>补充说明</button>
      {:else if comment.status === "clarify"}
        <button class="quiet sm" disabled={busy} onclick={resolve}>解决</button>
        <button class="primary sm" onclick={() => toggle("message")}>回复</button>
      {:else if comment.status === "resolved"}
        <button class="quiet sm" disabled={busy} onclick={() => toggle("reopen")}>重新打开</button>
      {:else}
        <button class="quiet sm" disabled={busy} onclick={() => toggle("reopen")}>重新打开</button>
        <button class="primary sm" disabled={busy} onclick={resolve}>解决</button>
      {/if}
    </div>
    {#if error}<p class="error small">{error}</p>{/if}
    {#if replying}
      <div class="reply" onclick={stop} onkeydown={stop} role="presentation">
        {#key replying}
          <Composer
            allowEmpty={replying === "reopen"}
            saveLabel={replying === "reopen" ? "重新打开" : "发送"}
            placeholder={replying === "reopen" ? "哪里还不对（可选），会进入下一轮" : comment.status === "clarify" ? "回答 AI 的问题，会进入下一轮" : "补充说明，会进入下一轮"}
            onsave={sendReply}
            oncancel={() => (replying = "")}
          />
        {/key}
      </div>
    {/if}
  {/if}
</div>
