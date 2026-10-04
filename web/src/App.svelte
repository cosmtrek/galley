<script lang="ts">
  import { onMount } from "svelte";
  import { get, onUnauthorized } from "./lib/api";
  import { router } from "./lib/router.svelte";
  import Login from "./routes/Login.svelte";
  import Reports from "./routes/Reports.svelte";
  import Workbench from "./routes/Workbench.svelte";
  import Verify from "./routes/Verify.svelte";
  import History from "./routes/History.svelte";
  import Publish from "./routes/Publish.svelte";

  let auth = $state<"checking" | "in" | "out">("checking");

  onUnauthorized(() => (auth = "out"));

  onMount(async () => {
    try {
      const me = await get<{ role: string }>("/api/me");
      auth = me.role === "owner" ? "in" : "out";
    } catch {
      auth = "out";
    }
  });

  const route = $derived(router.route);
</script>

{#if auth === "checking"}
  <div class="empty">加载中…</div>
{:else if auth === "out"}
  <Login onlogin={() => (auth = "in")} />
{:else if route.name === "reports"}
  <Reports />
{:else if route.name === "workbench"}
  {#key route.id}<Workbench id={route.id} />{/key}
{:else if route.name === "verify"}
  {#key route.id + (route.round ?? "")}<Verify id={route.id} roundId={route.round} />{/key}
{:else if route.name === "history"}
  {#key route.id}<History id={route.id} />{/key}
{:else if route.name === "publish"}
  {#key route.id}<Publish id={route.id} />{/key}
{:else}
  <div class="page"><h1>页面不存在</h1><a href="/app">返回报告列表</a></div>
{/if}
