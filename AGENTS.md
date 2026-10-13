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
- `pnpm build` 分两步：先打包工作台，再按 `vite.share.config.ts` 把分享页脚本打成自包含的 `dist/share.js`（服务端从 `web/dist` 嵌入，`cargo test` 也会检查它存在）。图片查看器 `web/src/lib/viewer.ts` 不依赖框架，两边共用，样式写在 `report.css`。
- 站点图标以 `server/static/favicon.svg` 为准，由服务端在 `/favicon.svg` 提供给工作台、分享页和状态页（无需登录）。`favicon.ico`（16/32/48）和 `apple-touch-icon.png`（180，白底）是从它导出的 PNG，改图形时要一起重新导出，并同步顶栏用的 `web/src/components/Logo.svelte`。
- 改动后按范围跑：

```sh
cd web && corepack pnpm check && corepack pnpm test && corepack pnpm build
cd server && cargo test && cargo build --release
```

- 本地开发用 `scripts/dev.sh` 编译和重启服务（默认子命令 `restart`：构建工作台和服务端，再重启 Galley；`status`、`check`、`stop`、`logs`、`tunnel` 见脚本开头）。它通过 cloudflared 临时隧道对外，重启 Galley 时保留隧道和地址，并把地址作为 `GALLEY_PUBLIC_URL` 传给服务；只有 `tunnel` 子命令或隧道已退出时才会换地址。运行状态（pid、隧道地址、cloudflared 日志）在 `data/dev/`。多个会话共用这一份服务，不要手动 `kill` 或 `nohup` 启动 galley，一律走这个脚本（脚本加了锁，并发执行会排队）。
- 使用 Rust 最新稳定版：`Dockerfile` 用浮动的 `rust:1-slim-bookworm`，CI 用 `stable`。最低版本是 1.99（`Cargo.toml` 的 `rust-version`，CI 会检查）；跟进新的稳定版时，同步改 `rust-version`、README 和 CI 的最低版本检查。
- 部署只提供 `Dockerfile` 和 `docker-compose.yml`（用户在本地 `docker compose up -d --build`），不发布镜像和二进制。改了构建步骤（依赖、目录、嵌入的文件）要确认 `docker build` 仍能通过。
- 服务端快照测试用 `insta`，快照在 `server/src/snapshots/`。输出确实应变时才更新快照，并在说明里写清原因。
- 改了工作台交互，要在真实浏览器里走一遍相关流程（选区、弹层、对话框等），不能只靠类型检查。

## 不能破坏的约定

- **偏移量单位**：服务端锚点偏移按 Unicode 码点计，DOM 按 UTF-16 计。前端所有换算都走 `web/src/lib/anchor.ts` 的辅助函数，不要直接用 `string.length` 或 DOM offset。
- **报告 DOM 不可改**：评论高亮用 CSS Custom Highlight API 和生成的 CSS 规则实现，不往 `version.html` 里插节点，否则锚点定位会错。
- **状态机集中在 `domain.rs`**：评论和轮次的状态流转、角色权限只在这里定义，并由穷举测试覆盖。新增动作或状态时同步更新 `comment_transition` / `round_transition` 和测试，前端按钮的可用条件要与之一致。
- **角色边界**：agent 不能解决或重新打开评论，owner 不能冒充 agent 提交结果。
- **分享页隐私**：`/s/<token>` 是服务端渲染的 HTML，不含评论、修订痕迹、版本历史和 `data-block` / `data-cell` 属性，带 `noindex`、`no-referrer`（`http_tests.rs` 有断言）。分享页和工作台同源，owner 打开时会带登录 cookie，所以 CSP 用每次响应随机的 nonce，只放行 `/static/share.js`（`web/src/share.ts`，目前只有图片查看器）这一个脚本，报告 HTML 里的任何脚本都不能执行；不要改成 `'self'` 或 `unsafe-inline`。这个脚本不发请求、不读评论相关数据，状态页保持 `script-src 'none'`。
- **附件**：上传文件按 sha256 存在 `<data>/assets/<report>/<sha256>`，写入后不再改动；版本渲染时把哈希写进 URL（`/a/<report>/<sha256>/<name>`），所以已发布的快照不会被同名重传改变。`/a/` 需要登录或 agent token；分享页把前缀改写为 `/s/<token>/a/`，只放行已发布版本 HTML 里引用的附件，撤销后返回 410。库里只存相对 `assets` 目录的路径。
- **HTML 安全**：Markdown 渲染结果由 `src/doc/parse.rs` 里的 ammonia 清洗，工作台直接 `{@html}` 渲染，放宽白名单前要评估 XSS 风险。
- **数据库迁移**：迁移按 `PRAGMA user_version` 顺序执行（`server/src/db.rs`）。已发布的迁移文件不改，新变更追加 `migrations/000N_*.sql` 并登记到 `MIGRATIONS`。
- **轮次完成条件**：没有 `verify` 状态的评论，且「评论之外的改动」全部确认（`maybe_complete_round`）。`completed_at` 只在轮次进入 `done` 时写入。
- **状态读取**：`load_round` 会把租约过期的 `processing` 显示为 `submitted`；任何状态变更都要从 `load_round_raw` 读取真实状态再走 `round_transition`。
- **错误信息**：`AppError::Internal` 的细节只写日志，返回给客户端的是 `internal error`（`client_message`）。
- **轮次结果原子性**：agent 提交的结果必须给每条待处理评论一条回复，否则整体拒绝；不要做部分应用。

## 代码风格

- 遵循周边代码：Rust 2024 edition；前端 Svelte 5 runes + TypeScript strict，样式写在 `web/src/styles/` 下的全局 CSS，沿用已有的 CSS 变量。
- 语言：产品界面（工作台、分享页、给用户看的错误提示）和对外文档（README）用中文；代码、标识符、注释、运行日志、提交信息、LICENSE 和包元数据（`Cargo.toml`、`package.json`）用英文。
- 注释只写代码本身看不出的原因（隐藏约束、浏览器怪癖、不变量）。

## 界面约定

- 按钮参照 shadcn/ui 的规范，定义在 `app.css` 的 buttons 一节，只用 class 组合，不单独给某个按钮写边框、高度或底色：
  - 变体：默认（次要，浅灰底 `--fill`）、`primary`（红色实心，一个区域最多一个，表示「轮到你处理」）、`ghost`（无底色，悬停出浅灰底；用于低优先级操作、取消、工具栏图标）、`danger`（红字浅红底，不可撤销的操作）、`link`（灰色带下划线的文字链接，不用红色）。
  - 尺寸：默认 32px（`--btn-h`）、`sm` 28px（`--btn-h-sm`）；纯图标按钮加 `icon`，是同高的正方形。同一行的按钮用同一尺寸。
  - 按钮不用边框区分，靠底色；圆角统一用 `--radius`。
  - 长得像按钮的链接用 `<a class="btn ...">`，和按钮共用变体与尺寸。
  - 图标用 `components/Icon.svelte`（16px 网格、描边、跟随 `currentColor`），新图标加到 `lib/icons.ts` 的 `PATHS` 里（图片查看器也用它），不要在页面里内联 SVG 或用「＋」「✕」之类的字符代替。
- 操作行用 `.actions`：靠右，次要在前、主操作在最右；「取消」用 `ghost`。
- 宽度只用 `app.css` 里的 `--container-*` 变量（Tailwind / shadcn 的容器刻度：3xs 256 目录、xs 320 评论栏、sm 384 登录和弹层、lg 512 确认框、2xl 672 对话框、5xl 1024 页面容器），不写一次性的 `max-width`。页面分三类：
  - 管理页（报告列表、版本历史、发布、404）：外层统一 `.page`（1024，居中），开头用 `PageHeader`（标题 + 说明 + 右侧操作），内容铺满容器，不在里面再限宽，保证各页标题位置一致、左右留白对称。
  - 阅读页（批注页、分享页）：目录 256，正文 70ch；批注页正文两侧留白尽量小，窗口变窄时先缩目录，不缩正文。分享页的 `server/static/share.css` 读不到这些变量，改刻度时要同步改它。
  - 单卡片页（登录、分享状态页）：居中窄卡片。
- 手机端以查看为主、编辑为辅，细节从简：批注页在窄屏（≤860px，`Workbench.svelte` 的 `narrow` 与 `app.css` 断点同步）用底栏显示当前状态，评论栏改为从底部弹出的面板，点卡片跳回原文时自动收起；任何操作都不能把页面滚走。触屏（`hover: none`）只保留选中文字评论，不显示段落左侧的评论按钮和批量勾选框，悬停才出现的按钮改为常显。
- 设置类页面（如发布页）：状态卡片用 `.panel.stack`，`.panel-head` 左边写当前状态，右边放唯一的主操作；单项设置用 `.panel.settings` 里的 `.setting` 行（标题 + 说明 + 右侧按钮）。不可撤销的操作集中放在页面最后的「不可撤销的操作」区，不和主操作放在一起。只影响某一块的提示写在那一块里，页面顶部的 banner 只留影响整页的状态（如已归档）。
- 红色只用于需要用户处理的状态（待验证色条、主按钮、待验证标签），计数、图标、选中态不用红色；选中态统一用 `--bg-active`。
- 确认用 `lib/confirm.svelte.ts` 的 `ask()`，不用原生 `confirm` / `alert` / `prompt`。只有不可撤销或批量的操作才确认。操作失败的提示显示在操作所在的区域。
- 加载状态不写「加载中…」文字，按场景用下面的方式（样式在 `app.css` 的 loading 一节）。所有指示都延迟 `--load-delay`（300ms）才出现，本地请求快时什么都不闪：
  - 首次加载：骨架屏。区域加 `.skeleton`，里面沿用真实页面的布局 class，文字行用 `Skeleton` 组件或 `.sk`（宽度用 `w25/w40/w60/w80`），保证内容到达时不跳动。标题、分组名等静态文字直接显示，只给数据部分画骨架。顶栏在报告未到时传 `loading`。登录态检查时布局未知，只显示居中的 Logo。
  - 首次加载失败：用 `LoadError`（说明 + 重试 + 返回列表），不要留一个永远的骨架；已有内容后的刷新失败才用页面顶部 banner。
  - 按钮操作：只给被点的那个按钮加 `aria-busy`（同时 `disabled`），文字换成同宽的转圈，不变淡；同一区域互斥的其他按钮只 `disabled`。用一个记录「哪个操作在跑」的状态（如 `busy: Action | null`），不要一个布尔值让所有按钮一起转。忙碌持续到随后的重新加载完成，避免按钮先恢复、列表后变化。
  - 局部刷新（如切换对比版本）：保留旧内容，加 `.refreshing` 变淡，新内容到了替换；有并发请求时只让最后一个生效。
  - 等 AI（等连接、等报告、等处理一轮）：状态点加 `.live` 呼吸，不用转圈，因为可能要等几分钟。后台轮询不显示任何指示。
  - 动画尊重 `prefers-reduced-motion`：骨架和呼吸点停止，转圈放慢。
- 复制用 `CopyButton`；评论类输入一律用 `Composer`（Enter 发送、Shift+Enter 换行、Esc 取消）。
- 字号只用 `--fs-xs/sm/md/lg/xl`，颜色只用 CSS 变量，不写行内样式（动态定位除外）。
- 用词：状态名与分组名一致（草稿、等 AI 处理、待验证……）；「恢复」只指取消归档，「回退」指回到旧版本，「改回去」指撤掉 AI 的额外改动，「停用」指作废分享链接；跳转箭头用「→」；完整句子结尾带句号。
