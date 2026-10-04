<script lang="ts">
  import { onMount } from "svelte";
  import { get, post } from "../lib/api";
  import { anchorLabel, ROUND_LABEL, STATUS_LABEL, truncate } from "../lib/format";
  import { navigate } from "../lib/router.svelte";
  import { diffCtx, partner, withoutPartners, type DiffCtx } from "../lib/diffctx";
  import type { BlockChange, ReportInfo, Review, ReviewItem, Round } from "../lib/types";
  import TopBar from "../components/TopBar.svelte";
  import ChangeView from "../components/ChangeView.svelte";
  import Thread from "../components/Thread.svelte";
  import PromptBox from "../components/PromptBox.svelte";

  let { id, roundId }: { id: string; roundId: string | null } = $props();

  let report = $state<ReportInfo | null>(null);
  let rounds = $state<Round[]>([]);
  let review = $state<Review | null>(null);
  let ctx = $state<DiffCtx | null>(null);
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
      const rd = review?.round;
      ctx = rd?.result_version_id ? await diffCtx(rd.base_version_id, rd.result_version_id).catch(() => null) : null;
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
  const extraShown = $derived(withoutPartners(ctx, extra));
  const extraPending = $derived(extraShown.filter((e) => !e.confirmed).length);

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
  const confirmAll = () => act("extra", () => post(`/api/rounds/${review!.round.id}/extra/confirm`, {}));
  // A replacement is displayed as one item, so acting on it covers both halves.
  function pairIds(ch: BlockChange) {
    const p = partner(ctx, ch);
    return [ch.block_id, ...(p && extra.some((e) => e.block_id === p.block_id && !e.confirmed) ? [p.block_id] : [])];
  }
  const confirmExtra = (ch: BlockChange) =>
    act("extra", async () => {
      for (const b of pairIds(ch)) await post(`/api/rounds/${review!.round.id}/extra/confirm`, { block_id: b });
    });

  const archived = $derived(!!report?.archived_at);
  const pendingNext = $derived(report ? report.counts.draft + report.counts.open : 0);
  // Right after submitting from here, the new round is what the user needs to act on.
  const nextRound = $derived(
    report?.active_round && review && report.active_round.seq === review.round.seq + 1 ? report.active_round : null,
  );
  async function submitNext() {
    if (!report || !confirm(`把 ${pendingNext} 条评论作为第 ${report.round_count + 1} 轮提交给 AI？提交后草稿不能再编辑。`)) return;
    await act("submit", () => post(`/api/reports/${id}/rounds`));
  }

  let reverting = $state<Record<string, string>>({});
  const revertDefault = (ch: BlockChange) =>
    partner(ctx, ch) || ch.op === "modified" || ch.op === "moved"
      ? "请恢复为修改前的内容。"
      : ch.op === "added"
        ? "这段是新增的，我不需要，请删掉。"
        : "这段被删掉了，请恢复。";
  const revertExtra = (ch: BlockChange) =>
    act("extra", async () => {
      await post(`/api/rounds/${review!.round.id}/extra/revert`, { block_ids: pairIds(ch), body: reverting[ch.block_id] });
      delete reverting[ch.block_id];
    });
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

    {#if archived}
      <div class="banner" style="margin-bottom: 16px">已归档，只读。要继续验证，请先到<a href="/app/r/{id}/publish">发布页</a>恢复。</div>
    {/if}

    {#if !archived && r.status === "done" && report && (nextRound || rounds[0]?.id === r.id)}
      <div class="next-step">
        {#if nextRound}
          <div class="row"><b>第 {nextRound.seq} 轮已提交</b><span class="muted small">{nextRound.comment_count} 条评论</span></div>
          {#if nextRound.status === "submitted"}
            <PromptBox {report} round={nextRound} />
          {:else}
            <div class="muted small">AI 已在处理第 {nextRound.seq} 轮，完成后回到这里验证。</div>
          {/if}
        {:else}
          <div class="row">
            <b>第 {r.seq} 轮已完成</b>
            <span class="spacer"></span>
            {#if pendingNext > 0}
              <span class="muted small">还有 {pendingNext} 条评论待交给 AI{#if report.counts.draft}（新写 {report.counts.draft} 条）{/if}</span>
              <button class="primary" disabled={busy === "submit"} onclick={submitNext}>提交第 {report.round_count + 1} 轮</button>
            {:else}
              <span class="muted small">所有评论都已处理</span>
              {#if report.publication && report.publication.version_id !== report.current_version_id}
                <a href="/app/r/{id}/publish">更新分享页（现在分享的还是 v{report.publication.version_seq}）→</a>
              {:else if report.publication}
                <span class="muted small">分享页已是最新</span>
              {:else}
                <a href="/app/r/{id}/publish">去发布 →</a>
              {/if}
            {/if}
          </div>
        {/if}
      </div>
    {:else if r.status === "verifying" && stat.verify === 0 && extraPending}
      <div class="next-step pending">
        <div>评论都处理完了，还有 <a href="#extra">{extraPending} 处评论之外的改动</a>待确认或改回去，处理完本轮才会完成。</div>
      </div>
    {/if}

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
            <span>{c.anchor.type === "text" ? "文字" : anchorLabel(c.anchor, it.section_title, it.change?.kind)}</span>
            <span class="spacer"></span>
            <span class="card-id mono">{c.id}</span>
          </div>
          <div class="vbody">
            <div>
              <div class="label">我的评论{#if c.original_quote && it.change?.kind !== "table"}（原文：{truncate(c.original_quote, 60)}）{/if}</div>
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
                <ChangeView change={it.change} {ctx} />
              {:else if it.section_changes.length}
                {#each withoutPartners(ctx, it.section_changes) as ch (ch.op + ch.block_id)}
                  <div style="margin-bottom: 8px"><ChangeView change={ch} {ctx} /></div>
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
          {#if archived}
            <!-- read-only -->
          {:else if c.status === "verify"}
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
      <div class="section-title row" id="extra">
        <span>评论之外的改动（{extraShown.length}）</span>
        <span class="spacer"></span>
        {#if extraPending && r.status === "verifying" && !archived}<button disabled={busy === "extra"} onclick={confirmAll}>全部确认</button>{/if}
      </div>
      <p class="muted small">这些地方改了，但本轮没有评论指向它们。逐条确认或改回去，全部处理后本轮才算完成。</p>
      {#each extraShown as ch (ch.op + ch.block_id)}
        <div class="vitem" class:done={ch.confirmed}>
          <div class="vhead">
            <span>{review.section_titles[ch.section_id] ?? "开头"}</span>
            <span class="spacer"></span>
            {#if ch.revert_comment_id}
              <a class="badge s-draft" href="/app/r/{id}">已转为草稿评论，下一轮改回</a>
            {:else if ch.confirmed}<span class="badge s-resolved">已确认</span>{/if}
          </div>
          <div class="vbody"><ChangeView change={ch} {ctx} /></div>
          {#if !ch.confirmed && r.status === "verifying" && !archived}
            <div class="actions">
              {#if ch.block_id in reverting}
                <textarea style="flex: 1; min-width: 240px" bind:value={reverting[ch.block_id]}></textarea>
                <button class="primary" disabled={busy === "extra" || !reverting[ch.block_id]?.trim()} onclick={() => revertExtra(ch)}>生成草稿评论</button>
                <button class="quiet" onclick={() => delete reverting[ch.block_id]}>取消</button>
              {:else}
                <button class="primary" disabled={busy === "extra"} onclick={() => confirmExtra(ch)}>确认</button>
                <button disabled={busy === "extra"} onclick={() => (reverting[ch.block_id] = revertDefault(ch))}>改回去</button>
                <span class="muted small">「改回去」会生成一条草稿评论（附上原文），随下一轮交给 AI。</span>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    {/if}

    <div class="row" style="margin-top: 32px">
      <a href="/app/r/{id}">← 回批注页写新评论</a>
      <span class="spacer"></span>
      {#if r.status === "verifying" && stat.verify === 0 && extraPending}
        <span class="muted">还有 {extraPending} 处评论之外的改动待处理，处理完本轮才会完成。</span>
      {:else if r.status === "done"}
        <span class="muted">本轮已完成。</span>
      {/if}
    </div>
  {/if}
</div>
