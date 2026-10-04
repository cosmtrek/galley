<script lang="ts">
  import { api, post } from "../lib/api";
  import { anchorLabel, blockSummary, STATUS_LABEL, truncate } from "../lib/format";
  import type { Block, Comment } from "../lib/types";
  import Composer from "./Composer.svelte";
  import Thread from "./Thread.svelte";

  let {
    comment,
    active = false,
    sectionTitle = null,
    block = null,
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
    inline?: boolean;
    pickable?: boolean;
    picked?: boolean;
    onpick?: (on: boolean) => void;
    onselect: () => void;
    onchanged: () => Promise<void> | void;
  } = $props();

  let editing = $state(false);
  let replying = $state<"" | "message" | "reopen">("");
  let text = $state("");
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
      replying = "";
      text = "";
      await onchanged();
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  const resolve = () => run(() => post(`/api/comments/${comment.id}/resolve`));
  const reopen = () => run(() => post(`/api/comments/${comment.id}/reopen`, { body: text }));
  const message = () => run(() => post(`/api/comments/${comment.id}/messages`, { body: text }));
  const remove = () => {
    if (confirm("删除这条草稿评论？")) run(() => api("DELETE", `/api/comments/${comment.id}`));
  };

  function stop(e: Event) {
    e.stopPropagation();
  }
</script>

<div
  class="card"
  class:active
  class:inline
  id={inline ? undefined : `card-${comment.id}`}
  onclick={onselect}
  onkeydown={(e) => e.key === "Enter" && e.target === e.currentTarget && onselect()}
  role="button"
  tabindex="0"
>
  <div class="card-head">
    {#if pickable}
      <input
        type="checkbox"
        class="pick"
        checked={picked}
        aria-label="选择这条评论"
        onclick={stop}
        onchange={(e) => onpick?.((e.currentTarget as HTMLInputElement).checked)}
      />
    {/if}
    <span class="badge s-{comment.status}">{STATUS_LABEL[comment.status]}</span>
    <span class="spacer"></span>
    <span class="card-id mono">{comment.id}</span>
  </div>

  {#if comment.anchor.type === "text"}
    <div class="quote">{truncate(comment.anchor.quote, 120)}</div>
  {:else}
    <div class="quote">{anchorLabel(comment.anchor, sectionTitle, block?.kind)}{#if quote}：{truncate(quote, 80)}{/if}</div>
  {/if}
  {#if changedQuote && block?.kind !== "table"}<div class="muted small">原文：{truncate(changedQuote, 80)}</div>{/if}
  {#if comment.anchor_state === "orphaned" && comment.status !== "resolved"}
    <div class="warn-note">锚定的内容已不在当前版本中</div>
  {:else if comment.anchor_state === "fuzzy" && comment.status !== "resolved"}
    <div class="warn-note">原文已改动，位置为近似匹配</div>
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
    <div class="body">{comment.body}</div>
  {/if}

  <Thread messages={comment.messages} />

  {#if !editing}
    <div class="actions" onclick={stop} onkeydown={stop} role="presentation">
      {#if comment.status === "draft"}
        <button class="link" onclick={() => (editing = true)}>编辑</button>
        <button class="link" onclick={remove}>删除</button>
      {:else if comment.status === "verify"}
        <button class="primary" disabled={busy} onclick={resolve}>解决</button>
        <button disabled={busy} onclick={() => (replying = replying === "reopen" ? "" : "reopen")}>重新打开</button>
      {:else if comment.status === "clarify"}
        <button class="primary" onclick={() => (replying = replying === "message" ? "" : "message")}>回复</button>
        <button disabled={busy} onclick={resolve}>直接解决</button>
      {:else if comment.status === "open"}
        <button class="link" onclick={() => (replying = replying === "message" ? "" : "message")}>补充说明</button>
      {:else if comment.status === "orphaned"}
        <button class="primary" disabled={busy} onclick={resolve}>解决</button>
        <button disabled={busy} onclick={() => (replying = replying === "reopen" ? "" : "reopen")}>重新打开</button>
      {:else if comment.status === "resolved"}
        <button class="link" disabled={busy} onclick={() => (replying = replying === "reopen" ? "" : "reopen")}>重新打开</button>
      {/if}
    </div>
    {#if replying}
      <div class="reply-box" onclick={stop} onkeydown={stop} role="presentation">
        <textarea bind:value={text} placeholder={replying === "reopen" ? "为什么重新打开（可选）" : "回复 AI"}></textarea>
        <div class="row">
          {#if replying === "reopen"}
            <button class="primary" disabled={busy} onclick={reopen}>重新打开</button>
          {:else}
            <button class="primary" disabled={busy || !text.trim()} onclick={message}>发送</button>
          {/if}
          <button onclick={() => (replying = "")}>取消</button>
        </div>
      </div>
    {/if}
  {/if}
  {#if error}<div class="error small">{error}</div>{/if}
</div>
