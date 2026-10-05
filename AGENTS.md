# Galley

AI 报告的批注、修订、验证与发布工具。Rust 单文件服务（axum + SQLite），内嵌 Svelte 5 工作台。功能、接口和运行方式见 [README](README.md)。

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

- 最低 Rust 版本是 1.88（`Cargo.toml` 的 `rust-version`，CI 会检查）；升级依赖导致要求变高时，同步改 `rust-version`、README 和 `Dockerfile` 的基础镜像。
- 部署只提供 `Dockerfile` 和 `docker-compose.yml`（用户在本地 `docker compose up -d --build`），不发布镜像和二进制。改了构建步骤（依赖、目录、嵌入的文件）要确认 `docker build` 仍能通过。
- 服务端快照测试用 `insta`，快照在 `server/src/snapshots/`。输出确实应变时才更新快照，并在说明里写清原因。
- 改了工作台交互，要在真实浏览器里走一遍相关流程（选区、弹层、对话框等），不能只靠类型检查。

## 不能破坏的约定

- **偏移量单位**：服务端锚点偏移按 Unicode 码点计，DOM 按 UTF-16 计。前端所有换算都走 `web/src/lib/anchor.ts` 的辅助函数，不要直接用 `string.length` 或 DOM offset。
- **报告 DOM 不可改**：评论高亮用 CSS Custom Highlight API 和生成的 CSS 规则实现，不往 `version.html` 里插节点，否则锚点定位会错。
- **状态机集中在 `domain.rs`**：评论和轮次的状态流转、角色权限只在这里定义，并由穷举测试覆盖。新增动作或状态时同步更新 `comment_transition` / `round_transition` 和测试，前端按钮的可用条件要与之一致。
- **角色边界**：agent 不能解决或重新打开评论，owner 不能冒充 agent 提交结果。
- **分享页隐私**：`/s/<token>` 是服务端渲染的纯 HTML，无 JS，不含评论、修订痕迹、版本历史和 `data-block` / `data-cell` 属性，带 `noindex`、`no-referrer` 和 `script-src 'none'` 的 CSP（`http_tests.rs` 有断言）。不要往分享页加脚本或评论相关数据。
- **附件**：上传文件按 sha256 存在 `<data>/assets/<report>/<sha256>`，写入后不再改动；版本渲染时把哈希写进 URL（`/a/<report>/<sha256>/<name>`），所以已发布的快照不会被同名重传改变。`/a/` 需要登录或 agent token；分享页把前缀改写为 `/s/<token>/a/`，只放行已发布版本 HTML 里引用的附件，撤销后返回 410。库里只存相对 `assets` 目录的路径。
- **HTML 安全**：Markdown 渲染结果由 `src/doc/parse.rs` 里的 ammonia 清洗，工作台直接 `{@html}` 渲染，放宽白名单前要评估 XSS 风险。
- **数据库迁移**：迁移按 `PRAGMA user_version` 顺序执行（`server/src/db.rs`）。已发布的迁移文件不改，新变更追加 `migrations/000N_*.sql` 并登记到 `MIGRATIONS`。
- **轮次完成条件**：没有 `verify` 状态的评论，且「评论之外的改动」全部确认（`maybe_complete_round`）。`completed_at` 只在轮次进入 `done` 时写入。
- **状态读取**：`load_round` 会把租约过期的 `processing` 显示为 `submitted`；任何状态变更都要从 `load_round_raw` 读取真实状态再走 `round_transition`。
- **错误信息**：`AppError::Internal` 的细节只写日志，返回给客户端的是 `internal error`（`client_message`）。
- **轮次结果原子性**：agent 提交的结果必须给每条待处理评论一条回复，否则整体拒绝；不要做部分应用。

## 代码风格

- 遵循周边代码：Rust 2024 edition；前端 Svelte 5 runes + TypeScript strict，样式写在 `web/src/styles/` 下的全局 CSS，沿用已有的 CSS 变量。
- 界面文案用中文，代码、标识符和注释用英文。
- 注释只写代码本身看不出的原因（隐藏约束、浏览器怪癖、不变量）。

## 界面约定

- 按钮三级：`primary`（红色实心，一个区域最多一个，表示「轮到你处理」）、次要（默认边框或 `quiet`）、`danger`（红色描边，不可撤销的操作）。小号用 `sm`。`button.link` 是灰色带下划线的文字链接，不用红色。
- 操作行用 `.actions`：靠右，次要在前、主操作在最右；「取消」用 `quiet`。
- 红色只用于需要用户处理的状态（待验证色条、主按钮、待验证标签），计数、图标、选中态不用红色；选中态统一用 `--bg-active`。
- 确认用 `lib/confirm.svelte.ts` 的 `ask()`，不用原生 `confirm` / `alert` / `prompt`。只有不可撤销或批量的操作才确认。操作失败的提示显示在操作所在的区域。
- 复制用 `CopyButton`；评论类输入一律用 `Composer`（Enter 发送、Shift+Enter 换行、Esc 取消）。
- 字号只用 `--fs-xs/sm/md/lg/xl`，颜色只用 CSS 变量，不写行内样式（动态定位除外）。
- 用词：状态名与分组名一致（草稿、等 AI 处理、待验证……）；「恢复」只指取消归档，「回退」指回到旧版本，「改回去」指撤掉 AI 的额外改动，「停用」指作废分享链接；跳转箭头用「→」；完整句子结尾带句号。
