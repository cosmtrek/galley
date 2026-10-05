# Galley

Galley is a self-hosted review workbench for AI-written reports. You comment on a Markdown report the way you would on a draft, hand the comments to your AI agent in one round, check what it changed, and publish a clean read-only link when you're done.

It is a single Rust binary with the web UI embedded; data lives in SQLite plus an assets directory. It is built for one person: one login password, plus one token for the AI agent. The UI is in Chinese.

## Features

- **Comment anywhere**: on selected text (across paragraphs too), a whole paragraph, table or diagram, a section, or the whole report.
- **Review in rounds**: drafts go to the agent together. The agent must reply to every comment (changed, answered, or a question back) and submits the revision atomically.
- **Verify in place**: each comment shows the agent's reply and a word-level diff. Edits no comment asked for are listed separately, to confirm or send back.
- **Comments follow the text**: blocks are aligned across versions and anchors relocated, so comments survive edits and moves; a comment whose text is gone is flagged instead of lost.
- **History**: compare any two versions and roll back.
- **Publish**: a snapshot behind an unguessable link, with no comments, revision marks or JavaScript. Republishing keeps the link; revoking kills it.
- **Diagrams**: ` ```mermaid ` and SVG blocks are rendered on the server, so they show on the share page too.
- **Agent access** over MCP or plain HTTP, with Bearer token auth. The agent can never resolve comments.

## Install

### Docker

```sh
git clone https://github.com/cosmtrek/galley.git && cd galley
docker compose up -d --build
docker compose logs galley   # the first start prints the generated password
```

Open http://localhost:7860/app. Data is kept in the `galley-data` volume. To upgrade, `git pull` and run `docker compose up -d --build` again.

### From source

Requires Rust 1.88+ and Node 24 (pnpm via corepack). The UI is embedded at compile time, so build it first:

```sh
cd web && corepack pnpm install && corepack pnpm build && cd ..
cd server && cargo build --release && cd ..
./server/target/release/galley
```

### Configuration

Set these as environment variables (in Docker, under `environment` in [`docker-compose.yml`](docker-compose.yml)):

| Variable | Default | Meaning |
| --- | --- | --- |
| `GALLEY_PUBLIC_URL` | `http://<addr>` | Address people and agents use to reach Galley; share links are built from it. An `https://` value also enables secure cookies |
| `GALLEY_PASSWORD` | generated | Login password |
| `GALLEY_AGENT_TOKEN` | generated | Bearer token for the AI agent |
| `GALLEY_ADDR` | `127.0.0.1:7860` (`0.0.0.0:7860` in Docker) | Listen address |
| `GALLEY_DATA` | `./data` (`/data` in Docker) | Database, uploads and generated secrets |

A password or token that is not set is generated on first start and saved to `<data>/secrets.json`. Galley serves plain HTTP; to reach it from other machines, put it behind a reverse proxy with TLS and set `GALLEY_PUBLIC_URL` to the `https://` address.

## Usage

1. **Connect your agent.** On the report list, open "接入方法" for the exact command for Claude Code, Codex, Droid or Devin, with your token filled in. For example:

   ```sh
   claude mcp add --transport http galley http://localhost:7860/mcp \
     --header "Authorization: Bearer <GALLEY_AGENT_TOKEN>"
   ```

   Agents without MCP can use the HTTP API: `GET /api/pending` lists rounds waiting for work, and `GET /api/rounds/<id>/packet?format=md` returns a round's comments with context and instructions. Images are uploaded with `POST /api/reports/<id>/assets?name=fig.png` (raw file as the body) and referenced as `![](assets/fig.png)`.

2. **Add a report.** Ask the agent to "把这份报告发到 Galley", or use "＋ 添加报告" → "手动导入" to paste Markdown or drop a `.md` file ("填入示例报告" loads a sample).
3. **Comment.** Select text, click "＋" in the left margin for a whole block, or use the outline for a section. Say what you want in one sentence, including whether it applies to the whole report.
4. **Submit the round.** Click "提交本轮", then "复制指令" and paste the instruction into your agent. You can keep writing drafts for the next round meanwhile.
5. **Verify.** When the agent is done, the sidebar shows each reply with its diff: resolve it, reopen it, or answer the agent's question. Confirm or send back edits listed under "评论之外的改动". Anything reopened goes into the next round.
6. **Publish.** On "发布", publish the current version and share the link.

## License

[MIT](LICENSE)
