<script lang="ts">
  import { tick } from "svelte";
  import { confirmState, settle } from "../lib/confirm.svelte";

  let dlg = $state<HTMLDialogElement | null>(null);
  let typed = $state("");
  const cur = $derived(confirmState.current);
  const ready = $derived(!cur?.requireText || typed.trim() === cur.requireText.trim());

  $effect(() => {
    if (!dlg) return;
    if (cur && !dlg.open) {
      typed = "";
      dlg.showModal();
      tick().then(() => dlg?.querySelector<HTMLElement>("input, button.confirm")?.focus());
    } else if (!cur && dlg.open) {
      dlg.close();
    }
  });

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (ready) settle(true);
  }
</script>

<dialog class="modal confirm" bind:this={dlg} oncancel={() => settle(false)}>
  {#if cur}
    <form onsubmit={submit}>
      <h2>{cur.title}</h2>
      {#if cur.message}<p class="lead">{cur.message}</p>{/if}
      {#if cur.requireText}
        <label class="field">
          <span class="muted small">输入「{cur.requireText}」确认</span>
          <input type="text" bind:value={typed} autocomplete="off" />
        </label>
      {/if}
      <div class="actions">
        <button type="button" class="quiet" onclick={() => settle(false)}>取消</button>
        <button type="submit" class="confirm {cur.danger ? 'danger' : 'primary'}" disabled={!ready}>{cur.confirmLabel ?? "确定"}</button>
      </div>
    </form>
  {/if}
</dialog>
