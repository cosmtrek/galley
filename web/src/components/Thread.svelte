<script lang="ts">
  import { ACTION_LABEL, fmtTime } from "../lib/format";
  import type { Message } from "../lib/types";

  let { messages }: { messages: Message[] } = $props();
  const shown = $derived(messages.filter((m) => m.body || m.action === "reopen"));
</script>

{#if shown.length}
  <div class="thread">
    {#each shown as m (m.id)}
      <div class="msg" class:agent={m.author === "agent"}>
        <span class="who">
          {m.author === "agent" ? "AI" : "我"}{#if m.action}&nbsp;· {ACTION_LABEL[m.action] ?? m.action}{/if} · {fmtTime(m.created_at)}
        </span>
        {m.body}
      </div>
    {/each}
  </div>
{/if}
