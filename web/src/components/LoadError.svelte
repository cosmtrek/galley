<script lang="ts">
  let { message, onretry, back = true }: { message: string; onretry: () => void; back?: boolean } = $props();
</script>

<div class="load-error" role="alert">
  <p class="title">没能加载出来</p>
  <p class="muted">{message}</p>
  <div class="row">
    <button class="sm" onclick={onretry}>重试</button>
    {#if back}<a class="btn ghost sm" href="/app">返回报告列表 →</a>{/if}
  </div>
</div>
