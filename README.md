# Galley

Galley is a review workbench for AI-written reports. You annotate a Markdown report, submit the comments as a round, an AI agent revises the report and replies to each comment, you verify the changes, and you publish a clean share link once you're done.

It ships as a single Rust binary. The Svelte workbench is embedded in it, and data lives in SQLite plus an assets directory.

## Run

Requirements: Rust 1.85+ and Node 20+ (pnpm through corepack, or npm).

```sh
# 1. Build the workbench. It is embedded at compile time, so build it before the server.
cd web && corepack pnpm install && corepack pnpm build && cd ..

# 2. Build and start the server.
cd server && cargo build --release && cd ..
GALLEY_PASSWORD=change-me ./server/target/release/galley
```

Then open http://127.0.0.1:7860/app and log in with the password.

| Variable | Default | Meaning |
| --- | --- | --- |
| `GALLEY_ADDR` | `127.0.0.1:7860` | Listen address |
| `GALLEY_DATA` | `./data` | SQLite database, uploaded assets, generated secrets |
| `GALLEY_PASSWORD` | generated | Owner login password |
| `GALLEY_AGENT_TOKEN` | generated | Bearer token for the AI agent |
| `GALLEY_PUBLIC_URL` | `http://<addr>` | Base URL used in share links; an `https://` value also enables secure cookies |

If the password or token is not set, Galley generates it on first start and stores it in `<data>/secrets.json` (mode 600).

To try it quickly, import `examples/energy-storage-2026.md` with the "导入 Markdown" button on the report list.

## Workflow

1. **Annotate (批注).** Switch the workbench between reading mode and comment mode with the "阅读 | 评论" toggle or the `M` key. In comment mode, select text and a comment box opens right below the pointer; use the "＋" in the left margin to comment on a whole block, or the outline to comment on a section or the whole report. A comment is one free-form sentence. Say what you want and, if it applies beyond this spot, say so ("全文类似的说法都改掉"). Enter saves, Shift+Enter adds a line. The right sidebar lists comments by status and supports batch resolve and batch delete of drafts.
2. **Submit the round (提交本轮).** Drafts become visible to the agent. You can keep writing drafts for the next round while the agent works.
3. **Agent revises.** The agent claims the round, reads the packet (comments with surrounding context, plus constraints from resolved comments), and submits the full new Markdown, one reply per comment, and a summary. A reply's action is `changed`, `answered`, or `clarify`.
4. **Verify (验证).** For each comment you see your comment, the agent's reply, and a word-level diff of the affected block. You can resolve it, reopen it with a reason, or answer a clarify question. Edits that no comment asked for are listed separately under "评论之外的改动" and need confirmation. Anything reopened or answered goes into the next round.
5. **History (历史).** Compare any two versions, or roll back. A rollback creates a new version.
6. **Publish (发布).** Publishing takes a snapshot of the current version and gives it an unguessable link (`/s/<token>`). The share page has no JavaScript and contains no comments, revision marks, or version history. It is served with `noindex` and `no-referrer` headers and a strict CSP. Republishing updates the same link. Revoking a link makes it return 410 permanently.

Comments follow the text across versions. Galley aligns the blocks of each new version with the previous one, so blocks keep their ids through edits, moves, and renumbered headings, and text anchors are relocated by quote and context. If an anchor's text disappears, the comment is marked orphaned instead of being silently dropped.

## Connecting an AI agent

The agent authenticates with `Authorization: Bearer <GALLEY_AGENT_TOKEN>`. It can do everything a round needs, but it cannot resolve or reopen comments.

### MCP

The streamable HTTP endpoint is `POST /mcp`. For Claude Code:

```sh
claude mcp add --transport http galley http://127.0.0.1:7860/mcp \
  --header "Authorization: Bearer $GALLEY_AGENT_TOKEN"
```

Codex reads the token from the environment at connect time:

```sh
codex mcp add galley --url http://127.0.0.1:7860/mcp --bearer-token-env-var GALLEY_AGENT_TOKEN
```

Droid:

```sh
droid mcp add galley http://127.0.0.1:7860/mcp --type http --no-oauth \
  --header "Authorization: Bearer $GALLEY_AGENT_TOKEN"
```

The server answers plain JSON over Streamable HTTP (no SSE stream, no sessions), so `GET /mcp` returns 405.

| Tool | Purpose |
| --- | --- |
| `galley_list_reports` | Reports and rounds waiting for the agent |
| `galley_create_report` | Import a new Markdown report |
| `galley_get_source` | Current Markdown source of a report |
| `galley_get_round` | Round packet: comments, context, constraints (claims the round) |
| `galley_submit_round` | Submit new Markdown, replies, and a summary in one atomic call |
| `galley_push_version` | Push a new version outside a round |

### HTTP

```sh
T="Authorization: Bearer $GALLEY_AGENT_TOKEN"
curl -H "$T" $URL/api/pending                                      # what needs work
curl -H "$T" -X POST $URL/api/rounds/<round>/claim                 # 30-minute lease
curl -H "$T" "$URL/api/rounds/<round>/packet?format=md"            # comments + context (or JSON without format)
curl -H "$T" "$URL/api/reports/<report>/source?format=raw"         # current Markdown
curl -H "$T" -H 'content-type: application/json' -X POST $URL/api/rounds/<round>/result \
  -d '{"markdown": "...", "summary": "...", "replies": [{"comment_id": "c_...", "action": "changed", "body": "..."}]}'
```

Other agent endpoints:

- `POST /api/reports` with `{"markdown": "..."}` creates a report.
- `POST /api/reports/<id>/versions` with `{"markdown": "...", "note": "..."}` pushes a version.
- `POST /api/reports/<id>/assets?name=fig1.png` with the raw file as the body uploads an image, which the report references as `![](assets/fig1.png)`.

Every pending comment must get a reply, or the result is rejected. Results are applied atomically.

## Development

```sh
cd server && cargo test                  # parsing, alignment, diff, anchors, state machines, HTTP + privacy
cd web && corepack pnpm test             # anchor helpers (vitest)
cd web && corepack pnpm check            # svelte-check
cd web && corepack pnpm dev              # Vite dev server on :5173, proxies the API to GALLEY_BACKEND (default :7860)
```

Layout:

- `server/src/doc`: Markdown to block model (pulldown-cmark, sanitized with ammonia).
- `server/src/align.rs`, `diff.rs`, `anchor.rs`: block identity across versions, version diffs, anchor relocation.
- `server/src/domain.rs`: round and comment state machines and role permissions.
- `server/src/store.rs`: SQLite storage (rusqlite); `migrations/` holds the schema.
- `server/src/api.rs`, `mcp.rs`, `share.rs`: HTTP API, MCP endpoint, public share page.
- `web/src`: Svelte 5 workbench (routes: Reports, Workbench, Verify, History, Publish).
