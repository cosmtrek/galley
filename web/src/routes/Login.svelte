<script lang="ts">
  import { post } from "../lib/api";

  let { onlogin }: { onlogin: () => void } = $props();
  let password = $state("");
  let error = $state("");
  let busy = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = "";
    try {
      await post("/api/login", { password });
      onlogin();
    } catch {
      error = "密码不对";
    } finally {
      busy = false;
    }
  }
</script>

<div class="login">
  <h1>Galley</h1>
  <p class="muted">AI 报告的校对与发布</p>
  <form onsubmit={submit}>
    <!-- svelte-ignore a11y_autofocus -->
    <input type="password" placeholder="密码" bind:value={password} autofocus autocomplete="current-password" />
    <button class="primary" disabled={busy || !password}>登录</button>
    {#if error}<div class="error">{error}</div>{/if}
  </form>
</div>
