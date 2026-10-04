<script lang="ts">
  import { ApiError, post } from "../lib/api";

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
    } catch (e) {
      error = e instanceof ApiError && e.status === 401 ? "密码不对" : (e as Error).message;
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
  <p class="muted small hint">
    首次启动时密码会打印在启动 Galley 的终端里，也保存在数据目录的 <code>secrets.json</code>（默认 <code>data/secrets.json</code>）。设置了
    <code>GALLEY_PASSWORD</code> 时以它为准。
  </p>
</div>
