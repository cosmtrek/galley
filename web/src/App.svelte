<script lang="ts">
  import { onMount } from "svelte";
  import { get, onUnauthorized } from "./lib/api";
  import { router } from "./lib/router.svelte";
  import Login from "./routes/Login.svelte";
  import Reports from "./routes/Reports.svelte";
  import Workbench from "./routes/Workbench.svelte";
  import History from "./routes/History.svelte";
  import Publish from "./routes/Publish.svelte";
  import ConfirmDialog from "./components/ConfirmDialog.svelte";
  import TopBar from "./components/TopBar.svelte";
  import PageHeader from "./components/PageHeader.svelte";
  import Logo from "./components/Logo.svelte";

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
  <!-- The page layout is unknown until auth settles (login or app), so only the mark shows. -->
  <div class="boot skeleton" role="status" aria-label="正在加载"><Logo /></div>
{:else if auth === "out"}
  <Login onlogin={() => (auth = "in")} />
{:else if route.name === "reports"}
  <Reports />
{:else if route.name === "workbench"}
  {#key route.id}<Workbench id={route.id} />{/key}
{:else if route.name === "history"}
  {#key route.id}<History id={route.id} />{/key}
{:else if route.name === "publish"}
  {#key route.id}<Publish id={route.id} />{/key}
{:else}
  <TopBar />
  <div class="page">
    <PageHeader title="页面不存在" />
    <a href="/app">返回报告列表 →</a>
  </div>
{/if}
<ConfirmDialog />
