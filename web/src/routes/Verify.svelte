<script lang="ts">
  import { onMount } from "svelte";
  import { get, post } from "../lib/api";
  import { anchorLabel, ROUND_LABEL, STATUS_LABEL, truncate } from "../lib/format";
  import { navigate } from "../lib/router.svelte";
  import type { ReportInfo, Review, ReviewItem, Round } from "../lib/types";
  import TopBar from "../components/TopBar.svelte";
  import ChangeView from "../components/ChangeView.svelte";
  import Thread from "../components/Thread.svelte";

  let { id, roundId }: { id: string; roundId: string | null } = $props();

  let report = $state<ReportInfo | null>(null);
  let rounds = $state<Round[]>([]);
  let review = $state<Review | null>(null);
  let error = $state("");
  let notes = $state<Record<string, string>>({});
  let reopening = $state<Record<string, boolean>>({});
  let busy = $state<string | null>(null);

  async function load() {
    try {
      report = (await get<{ report: ReportInfo }>(`/api/reports/${id}`)).report;
      rounds = await get<Round[]>(`/api/reports/${id}/rounds`);
      const target =
        roundId ??
        rounds.find((r) => r.status === "verifying")?.id ??
        rounds.find((r) => r.status === "done")?.id ??
        null;
      review = target ? await get<Review>(`/api/rounds/${target}/review`) : null;
      error = "";
    } catch (e) {
      error = (e as Error).message;
    }
  }
  onMount(load);

  const items = $derived(review?.items ?? []);
  const groups = $derived.by(() => {
    const out: { key: string; title: string; items: ReviewItem[] }[] = [];
    for (const it of items) {
      const key = it.comment.anchor.type === "document" ? "__doc" : (it.comment.section_id ?? "__none");
      const title =
        key === "__doc" ? "整篇报告" : key === "__none" ? "开头" : (review?.section_titles[key] ?? it.section_title ?? "");
      let g = out.find((x) => x.key === key);
      if (!g) out.push((g = { key, title, items: [] }));
      g.items.push(it);
    }
    return out;
  });
  const stat = $derived({
    verify: items.filter((i) => i.comment.status === "verify").length,
    clarify: items.filter((i) => i.comment.status === "clarify").length,
    resolved: items.filter((i) => i.comment.status === "resolved").length,
    reopened: items.filter((i) => i.comment.status === "open").length,
    orphaned: items.filter((i) => i.comment.status === "orphaned").length,
  });
  const decided = $derived(items.length - stat.verify);
  const extra = $derived(review?.round.extra_changes ?? []);
  const extraPending = $derived(extra.filter((e) => !e.confirmed).length);

  function lastAgent(it: ReviewItem) {
    const ms = it.comment.messages.filter((m) => m.author === "agent" && m.round_id === review?.round.id);
    return ms[ms.length - 1] ?? null;
  }

  async function act(cid: string, fn: () => Promise<unknown>) {
    busy = cid;
    try {
      await fn();
      await load();
    } catch (e) {
      alert((e as Error).message);
    } finally {
      busy = null;
    }
  }

  const resolve = (cid: string) => act(cid, () => post(`/api/comments/${cid}/resolve`));
  const reopen = (cid: string) =>
    act(cid, async () => {
      await post(`/api/comments/${cid}/reopen`, { body: notes[cid] ?? "" });
      reopening[cid] = false;
      notes[cid] = "";
    });
  const reply = (cid: string) =>
    act(cid, async () => {
      await post(`/api/comments/${cid}/messages`, { body: notes[cid] ?? "" });
      notes[cid] = "";
    });
  const confirmExtra = (blockId: string | null) =>
    act("extra", () => post(`/api/rounds/${review!.round.id}/extra/confirm`, blockId ? { block_id: blockId } : {}));
</script>

<TopBar {report} active="verify" />

{#if error}<div class="banner attention">{error}</div>{/if}

<div class="verify">
  {#if !review}
    {#if report}
      <h1 style="font-size: 20px">没有待验证的轮次</h1>
      <p class="muted">
        {#if report.active_round}第 {report.active_round.seq} 轮{ROUND_LABEL[report.active_round.status]}。{:else}先在批注页写评论并提交本轮。{/if}
      </p>
      <a href="/app/r/{id}">← 回到批注</a>
    {/if}
  {:else}
    {@const r = review.round}
    <div class="row" style="margin-bottom: 12px">
      <h1 style="font-size: 20px; margin: 0">第 {r.seq} 轮验证</h1>
      <span class="badge" class:s-verify={r.status === "verifying"}>{ROUND_LABEL[r.status]}</span>
      <span class="muted">v{review.base_seq} → {review.result_seq ? `v${review.result_seq}` : "未出新版本"}</span>
      <span class="spacer"></span>
      {#if rounds.length > 1}
        <select style="width: auto" value={r.id} onchange={(e) => navigate(`/app/r/${id}/verify?round=${(e.target as HTMLSelectElement).value}`)}>
          {#each rounds as x (x.id)}
            <option value={x.id}>第 {x.seq} 轮 · {ROUND_LABEL[x.status]}</option>
          {/each}
        </select>
      {/if}
    </div>

    <div class="summary-box">
      <div class="label muted small">AI 本轮摘要</div>
      <div class="ai">{r.summary || "（AI 没有写摘要）"}</div>
      <div class="stats">
        <div class="stat"><b>{stat.verify}</b><span>待验证</span></div>
        <div class="stat"><b>{stat.clarify}</b><span>待澄清</span></div>
        <div class="stat"><b>{stat.resolved}</b><span>已解决</span></div>
        <div class="stat"><b>{stat.reopened}</b><span>重新打开</span></div>
        {#if stat.orphaned}<div class="stat"><b>{stat.orphaned}</b><span>已失效</span></div>{/if}
        <div class="stat"><b>{extraPending}</b><span>评论之外的改动待确认</span></div>
      </div>
      <div class="progress"><div style="width: {items.length ? (100 * decided) / items.length : 0}%"></div></div>
    </div>

    {#each groups as g (g.key)}
      <div class="section-title">{g.title}</div>
      {#each g.items as it (it.comment.id)}
        {@const c = it.comment}
        {@const agent = lastAgent(it)}
        <div class="vitem" class:done={c.status === "resolved"}>
          <div class="vhead">
            <span class="badge s-{c.status}">{STATUS_LABEL[c.status]}</span>
            <span>{c.anchor.type === "text" ? "文字" : anchorLabel(c.anchor, it.section_title)}</span>
            <span class="spacer"></span>
            <span class="mono">{c.id}</span>
          </div>
          <div class="vbody">
            <div>
              <div class="label">我的评论{#if c.original_quote}（原文：{truncate(c.original_quote, 60)}）{/if}</div>
              <div style="white-space: pre-wrap">{c.body}</div>
            </div>
            {#if agent}
              <div>
                <div class="label">AI 回复 · {agent.action === "changed" ? "已修改" : agent.action === "answered" ? "已回答" : "有疑问"}</div>
                <div style="white-space: pre-wrap">{agent.body}</div>
              </div>
            {/if}
            <div>
              <div class="label">修改对比</div>
              {#if it.change}
                <ChangeView change={it.change} />
              {:else if it.section_changes.length}
                {#each it.section_changes as ch (ch.block_id)}
                  <div style="margin-bottom: 8px"><ChangeView change={ch} /></div>
                {/each}
              {:else if c.anchor.type === "document"}
                <div class="muted small">整篇评论：改动分布在全文，可在「历史」中查看 v{review.base_seq} → v{review.result_seq ?? review.base_seq} 的完整对比。</div>
              {:else}
                <div class="muted small">锚定位置没有文字改动。</div>
              {/if}
            </div>
            {#if c.messages.filter((m) => m.round_id !== r.id && m.body).length}
              <details>
                <summary class="muted small">之前的对话</summary>
                <Thread messages={c.messages.filter((m) => m.round_id !== r.id)} />
              </details>
            {/if}
          </div>
          {#if c.status === "verify"}
            <div class="actions">
              <button class="primary" disabled={busy === c.id} onclick={() => resolve(c.id)}>解决</button>
              {#if reopening[c.id]}
                <textarea style="flex: 1; min-width: 240px" bind:value={notes[c.id]} placeholder="哪里还不对（可选），会进入下一轮"></textarea>
                <button disabled={busy === c.id} onclick={() => reopen(c.id)}>确认重新打开</button>
                <button class="quiet" onclick={() => (reopening[c.id] = false)}>取消</button>
              {:else}
                <button disabled={busy === c.id} onclick={() => (reopening[c.id] = true)}>重新打开</button>
              {/if}
            </div>
          {:else if c.status === "clarify"}
            <div class="actions">
              <textarea style="flex: 1; min-width: 240px" bind:value={notes[c.id]} placeholder="回答 AI 的问题，会进入下一轮"></textarea>
              <button class="primary" disabled={busy === c.id || !notes[c.id]?.trim()} onclick={() => reply(c.id)}>回复</button>
              <button disabled={busy === c.id} onclick={() => resolve(c.id)}>直接解决</button>
            </div>
          {:else if c.status === "orphaned"}
            <div class="actions">
              <span class="muted small" style="flex: 1">锚定的内容已被删除。</span>
              <button class="primary" disabled={busy === c.id} onclick={() => resolve(c.id)}>解决</button>
              <button disabled={busy === c.id} onclick={() => reopen(c.id)}>重新打开</button>
            </div>
          {/if}
        </div>
      {/each}
    {/each}

    {#if extra.length}
      <div class="section-title row">
        <span>评论之外的改动（{extra.length}）</span>
        <span class="spacer"></span>
        {#if extraPending && r.status === "verifying"}<button disabled={busy === "extra"} onclick={() => confirmExtra(null)}>全部确认</button>{/if}
      </div>
      <p class="muted small">这些块发生了变化，但没有被本轮任何评论锚定。确认它们是你想要的改动，全部确认后本轮才算完成。</p>
      {#each extra as ch (ch.block_id)}
        <div class="vitem" class:done={ch.confirmed}>
          <div class="vhead">
            <span>{review.section_titles[ch.section_id] ?? "开头"}</span>
            <span class="spacer"></span>
            {#if ch.confirmed}<span class="badge s-resolved">已确认</span>{/if}
          </div>
          <div class="vbody"><ChangeView change={ch} /></div>
          {#if !ch.confirmed && r.status === "verifying"}
            <div class="actions">
              <button disabled={busy === "extra"} onclick={() => confirmExtra(ch.block_id)}>确认</button>
              <span class="muted small">不想要的话，回批注页对这段写一条评论，下一轮让 AI 改回去。</span>
            </div>
          {/if}
        </div>
      {/each}
    {/if}

    <div class="row" style="margin-top: 32px">
      <a href="/app/r/{id}">← 回批注页写新评论</a>
      <span class="spacer"></span>
      {#if r.status === "verifying" && stat.verify === 0 && extraPending}
        <span class="muted">还有 {extraPending} 处评论之外的改动待确认，确认后本轮才会完成。</span>
      {:else if r.status === "done"}
        <span class="muted">本轮已完成。</span>
        {#if report && report.counts.draft + report.counts.open > 0}
          <a href="/app/r/{id}">提交下一轮 →</a>
        {:else}
          <a href="/app/r/{id}/publish">去发布 →</a>
        {/if}
      {/if}
    </div>
  {/if}
</div>
