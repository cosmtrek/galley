<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { truncate } from "../lib/format";

  let {
    targetLabel = null,
    quote = null,
    initialBody = "",
    saveLabel = "保存",
    allowEmpty = false,
    placeholder = "想怎么改，一句话说清楚。比如：这个数字请核实并补来源；全文类似的说法都改掉",
    dirty = $bindable(false),
    onsave,
    oncancel,
  }: {
    targetLabel?: string | null;
    quote?: string | null;
    initialBody?: string;
    saveLabel?: string;
    /** Reopening takes an optional reason, so an empty body may be saved. */
    allowEmpty?: boolean;
    placeholder?: string;
    dirty?: boolean;
    onsave: (body: string) => Promise<void>;
    oncancel: () => void;
  } = $props();

  // The composer edits a copy; later prop changes must not clobber what is being typed.
  let body = $state(untrack(() => initialBody));
  let busy = $state(false);
  let error = $state("");
  let textarea: HTMLTextAreaElement;

  onMount(() => textarea?.focus({ preventScroll: true }));

  $effect(() => {
    dirty = body.trim() !== untrack(() => initialBody).trim();
  });

  async function save() {
    if ((!allowEmpty && !body.trim()) || busy) return;
    busy = true;
    error = "";
    try {
      await onsave(body);
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  function keydown(e: KeyboardEvent) {
    // Enter confirms an IME candidate while composing Chinese; only a bare Enter afterwards saves.
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing && e.keyCode !== 229) {
      e.preventDefault();
      save();
    } else if (e.key === "Escape") {
      // Otherwise the same keypress also closes the confirm dialog that cancelling may open.
      e.preventDefault();
      oncancel();
    }
  }
</script>

<div class="composer">
  {#if targetLabel}
    <div class="target">
      {targetLabel}{#if quote}：<q>{truncate(quote, 60)}</q>{/if}
    </div>
  {/if}
  <textarea bind:this={textarea} bind:value={body} onkeydown={keydown} {placeholder} rows="2"></textarea>
  {#if error}<p class="error small">{error}</p>{/if}
  <div class="actions">
    <button class="quiet sm" onclick={oncancel}>取消</button>
    <button class="primary sm" disabled={(!allowEmpty && !body.trim()) || busy} onclick={save}>{saveLabel}</button>
  </div>
</div>
