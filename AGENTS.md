# Galley

AI 报告的批注、修订、验证与发布工具。Rust 单文件服务（axum + SQLite），内嵌 Svelte 5 工作台。功能、接口和运行方式见 [README](README.md)；产品与技术方案原文在 `/root/ryos/projects/galley/`（`README.md`、`architecture.md`）。

## 沟通

- 没有特殊要求时，默认用中文回复。

## 目录

- `server/`：后端。`src/doc` 解析 Markdown 为块模型；`align.rs`、`diff.rs`、`anchor.rs` 负责跨版本块对齐、对比和锚点重定位；`domain.rs` 是评论与轮次状态机；`store.rs` 是存储；`api.rs`、`mcp.rs`、`share.rs` 是 HTTP、MCP 和分享页。
- `web/`：工作台，路由在 `src/routes`，组件在 `src/components`，选区与锚点换算在 `src/lib/anchor.ts`。
- `examples/`：试用用的示例报告。
- `data/`：本地运行数据（SQLite、assets、`secrets.json`），已被忽略，不提交、不清理、不打印其中的密码和 token。

## 构建与验证

- 工作台在编译期嵌入二进制（`rust-embed` 读取 `web/dist`），改了 `web/` 必须先 `pnpm build` 再 `cargo build`，否则服务仍在提供旧页面。
- `web/src/styles/report.css` 也被服务端 `include_str!` 进分享页，改它会同时影响工作台和分享页，改完要重新编译服务端。
- 改动后按范围跑：

```sh
cd web && corepack pnpm check && corepack pnpm test && corepack pnpm build
cd server && cargo test && cargo build --release
```

- 服务端快照测试用 `insta`，快照在 `server/src/snapshots/`。输出确实应变时才更新快照，并在说明里写清原因。
- 改了工作台交互，要在真实浏览器里走一遍相关流程（选区、弹层、对话框等），不能只靠类型检查。

## 不能破坏的约定

- **偏移量单位**：服务端锚点偏移按 Unicode 码点计，DOM 按 UTF-16 计。前端所有换算都走 `web/src/lib/anchor.ts` 的辅助函数，不要直接用 `string.length` 或 DOM offset。
- **报告 DOM 不可改**：评论高亮用 CSS Custom Highlight API 和生成的 CSS 规则实现，不往 `version.html` 里插节点，否则锚点定位会错。
- **状态机集中在 `domain.rs`**：评论和轮次的状态流转、角色权限只在这里定义，并由穷举测试覆盖。新增动作或状态时同步更新 `comment_transition` / `round_transition` 和测试，前端按钮的可用条件要与之一致。
- **角色边界**：agent 不能解决或重新打开评论，owner 不能冒充 agent 提交结果。
- **分享页隐私**：`/s/<token>` 是服务端渲染的纯 HTML，无 JS，不含评论、修订痕迹、版本历史和 `data-block` / `data-cell` 属性，带 `noindex`、`no-referrer` 和 `script-src 'none'` 的 CSP（`http_tests.rs` 有断言）。不要往分享页加脚本或评论相关数据。
- **HTML 安全**：Markdown 渲染结果由 `src/doc/parse.rs` 里的 ammonia 清洗，工作台直接 `{@html}` 渲染，放宽白名单前要评估 XSS 风险。
- **数据库迁移**：迁移按 `PRAGMA user_version` 顺序执行（`server/src/db.rs`）。已发布的迁移文件不改，新变更追加 `migrations/000N_*.sql` 并登记到 `MIGRATIONS`。
- **轮次结果原子性**：agent 提交的结果必须给每条待处理评论一条回复，否则整体拒绝；不要做部分应用。

## 代码风格

- 遵循周边代码：Rust 2024 edition；前端 Svelte 5 runes + TypeScript strict，样式写在 `web/src/styles/` 下的全局 CSS，沿用已有的 CSS 变量。
- 界面文案用中文，代码、标识符和注释用英文。
- 注释只写代码本身看不出的原因（隐藏约束、浏览器怪癖、不变量）。
