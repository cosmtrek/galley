<script lang="ts">
  import { copyText } from "../lib/clipboard";

  let {
    text,
    label = "复制",
    disabled = false,
    small = false,
    failed = $bindable(false),
  }: { text: string; label?: string; disabled?: boolean; small?: boolean; failed?: boolean } = $props();

  let state = $state<"" | "ok" | "fail">("");
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function copy() {
    const ok = await copyText(text);
    state = ok ? "ok" : "fail";
    failed = !ok;
    clearTimeout(timer);
    timer = setTimeout(() => (state = ""), 2000);
  }
</script>

<button class:sm={small} {disabled} onclick={copy}>{state === "ok" ? "已复制" : state === "fail" ? "复制失败" : label}</button>
