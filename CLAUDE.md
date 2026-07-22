<!-- codebase-memory-mcp knowledge graph -->

## MCP Tools: codebase-memory-mcp

**IMPORTANT: This project has a knowledge graph (project `C-data-codes-mica-will-laptop`).
ALWAYS use the codebase-memory-mcp tools BEFORE Grep/Glob/Read to explore
the codebase.** The graph is faster, cheaper (fewer tokens), and gives you
structural context (callers, callees, data flow, tests) that file scanning
cannot. (code-review-graph was uninstalled 2026-07-15 — this is now the ONLY
code graph; ignore any lingering references to it.)

### When to use graph tools FIRST

- **Find a symbol** (function/class/route): `search_graph` (name_pattern / label / qn_pattern) instead of Grep
- **Text search across code**: `search_code` (graph-augmented grep) instead of raw Grep
- **Read one symbol's source**: `get_code_snippet` (qualified_name) — precise range, cheaper than Read
- **Call chains / impact**: `trace_path` (mode=calls|data_flow|cross_service) instead of hand-tracing imports
- **Complex relationships**: `query_graph` (Cypher — callers/callees/imports/tests)
- **Architecture questions**: `get_architecture` (aspects) instead of reading many files
- **Code review**: `detect_changes` for a risk-scored diff analysis

Fall back to Grep/Glob/Read **only** when the graph doesn't cover what you need.

### Key Tools

| Tool | Use when |
| ------ | ---------- |
| `search_graph` | Find functions/classes/routes by name / label / qualified-name |
| `search_code` | Graph-augmented text search (grep replacement) |
| `get_code_snippet` | Exact source of a symbol by qualified name |
| `trace_path` | Call chains / data flow / cross-service paths |
| `query_graph` | Cypher patterns — callers, callees, imports, tests |
| `get_architecture` | High-level structure (e.g. aspects=['all']) |
| `detect_changes` | Risk-scored review of what changed |
| `index_repository` | (Re)index — MANUAL, see Workflow #1 |
| `index_status` / `list_projects` | Check freshness / list indexed projects |

### Workflow

1. **The graph is NOT auto-updated** (no file-change hook — that was code-review-graph's).
   After a substantial change, or if `search_graph` misses a just-added symbol, re-run
   `index_repository(repo_path="C:/data/codes/mica-will-laptop", mode="full")`. Modes: `full` (incl.
   similarity/semantic edges, slow) / `moderate` / `fast`. Check freshness with `index_status`.
   查询要带 `project="C-data-codes-mica-will-laptop"`(项目名由路径推导)。`full` 模式偶发
   worker 崩溃(崩溃日志是空文件),原样重试一次即可。
2. Use `search_graph` / `search_code` / `get_code_snippet` for exploration.
3. Use `trace_path` / `query_graph` for callers/callees/impact and test coverage.
4. Use `detect_changes` for code review.

<!-- Project principles & state — distilled from the Linux dev machine's agent
     memory (2026-06-06) so any session on any machine starts with them. -->

## 项目原则(长期有效,优先级高于默认习惯)

1. **In-house 优先**:最小化第三方依赖,宁可自研(编辑器整个是自绘的)。引入依赖需明确豁免(现有豁免:#1 flutter_math_fork——数学渲染;#2 window_manager——桌面窗口大小/位置/最小尺寸/拦截关闭;#8 tray_manager——系统托盘("关闭最小化到托盘"用)。window_manager **完全没有** tray API(它自己的 example 都得额外引 tray_manager 才能演示托盘),自研等于手写 Win32 `Shell_NotifyIcon` + Linux StatusNotifier/DBus 两套原生层,正是本条判定"用成熟包反而对"的粘合层;与 window_manager 同作者同发布者(leanflutter.dev)。**仅 Windows 启用**(`trayIsSupported`):Linux 上 libayatana-appindicator3 是构建+运行硬依赖(缺库启动即崩)、Debian 13/新 Ubuntu 上 deprecated API 撞 `-Werror` 编译失败、GNOME 还需用户装扩展——任一条都会让"隐藏到托盘"变成"窗口再也找不回来";CI 也只在 windows-latest 上构建 Flutter 桌面包。托盘注册失败必须降级为 `minimize()`,**绝不 hide**;#3 file_picker——桌面文件对话框(开/存/目录);#4 pasteboard——桌面富剪贴板图片读写;#5 merman——纯 Rust headless mermaid 引擎(FFI,桌面/移动离线渲 mermaid,同 flutter_math_fork 类的复杂领域渲染器,自研不现实);#6 flutter_svg——把 merman 的 SVG 栅格成 ui.Image;#7 yjs(JS 库,web-only)——web 端 CRDT 引擎,是 Rust `yrs` 核心的 JS 对端,**和 yrs 在 update/state-vector/lib0 v1 编码层字节兼容**(已实证),web 跑不了我们的 Rust FFI 故用它实现"一个权威引擎全平台跑";仅 web bundle(`tool/yjs` esbuild 打成 `web/yjs_bundle.js`),不入桌面;另 `xml`/`html` 为纯 Dart 解析库。除 flutter_math_fork/yjs 外均只在非 web 的 `_stub`/`_web` 变体里用,条件导入隔离不入 web bundle。均属标准边角/平台粘合层或复杂领域渲染器,自研要背三套平台原生层或重写一个引擎,用成熟包反而对)。in-house 该留给核心数据面(CRDT/文档模型/同步),不是平台粘合层。merman 的 mermaid SVG 主题用 CSS,纯 Dart 渲染器不解析 → 自研了 `mermaid_svg_inline.dart` 把 CSS 拍平进属性(merman 文档把这列为 host 边界)。
2. **Rust-first 数据面**:数据处理一律在 Rust 后端;Dart 只做 UI 和编辑器热路径。Markdown 语法逻辑两端必须同步(Rust `crates/markdown` 是权威,Dart `lib/editor/marks.dart`/`markdown.dart` 镜像)。
3. **渲染架构红线**:新渲染能力先抽象机制(AtomicBlockRenderer 注册表),严禁往 `render.dart` 堆 if 分支。见 `docs/render-architecture.md`。
4. **Markdown 方言原则**:CommonMark 0.31.2 底座(读侧 641/641=100%)+ GFM 扩展(24/24)+ 方言(脚注、front matter、Pandoc 数学约定);写侧输出规范化子集,round-trip 是不变量。记分牌:`docs/commonmark-scoreboard.md`,回归地板在 `commonmark_scoreboard.rs`。
5. **修复纪律**:每个 bug 修复配回归测试 + 实测验证(web 端用 playwright-cli 截图);提交信息写根因,不写流水账。
6. **难决策先调研同类产品**:面对没有明显正确/完美方案的架构或技术决策(或自己的方案有明显代价/取舍)时,拍板前先去同类开源/闭源产品扒一遍实现,当作兜底手段——去别人那里找灵感,而不是凭自己的假设硬推。重点不是"它支不支持",而是"在和我们**相同约束**下它具体怎么实现、又刻意**没用什么**"(排除法往往最有信息量;闭源就从 pubspec/依赖清单、CHANGELOG、issue 等公开痕迹推断)。最该警惕的是自己脑子里"必须 X 才能 Y"那类前提,正是它最值得被别人的实现证伪——给出"几选一"前先自问这些选项的**共同前提**验证过没有。手段:派调研子代理 + GitHub MCP 读对方真实源码。本项目参照系:AppFlowy(Flutter 原生同构)、AFFiNE(web/Yjs 对照)。〔教训:mermaid 桌面渲染曾基于"服务端渲 mermaid 必须 headless 浏览器"的错误前提,差点选 Kroki/Node/Chrome;扒 AppFlowy+AFFiNE 后才发现纯 Rust 渲染器(mermaid-rs-renderer/merman)这条离线+跨平台+无浏览器的更优路径。〕

## 当前状态(2026-06-06)

- **Web 端功能稳定**,Markdown 规范线已闭环。
- **桌面端启动**:Flutter Desktop 原生,Windows 优先。完整决策与 M1-M3 计划在 **`docs/desktop-plan.md`**(新会话先读它)。
- 图片存储:image block 存 `file_id`(非 URL),sha256 去重,加载时解析 URL;导出保留原文件名。
- 开发环境备忘(stale bundle、幽灵会话、DB 取证)也在 `docs/desktop-plan.md`。

## 架构速记

- 编辑器:单 RenderBox 自绘画布(`render.dart`),marks-over-plain-text 模型,IME 走 TextInputClient;硬换行存储约定 = 文本里的 `\`+换行。
- **页树不变量:folder 是唯一容器,page 是叶子**。以前这条只活在 Dart 客户端(`models.dart: canNestUnder`),服务端随便造——Notion 导入就造出了 137 个"页面下挂页面"。现在服务端任何写 `parent_view_id` 的路径**必须**过 `documents::ensure_parent_accepts_children`(400 + 可读原因),DB 侧 `views_parent_must_be_folder` 触发器兜底(migration 0011,同时修复了存量)。
- 块级嵌套是扁平模型:`data.indent`(列表层级)、`data.quote`/`qbreak`(引用深度/分组)、`data.li`(item 容器子块),HTML 导出端重建嵌套。
- 代码字体:web 上 `'monospace'` 族名不解析,一律用 `kMonoFont`(打包的 Roboto Mono,`model.dart`)。
- 发版/构建见 **`docs/release.md`**(权威):Actions 只出安装包 + CLI(推 `v*` tag 触发);web/api 全靠本地 `just deploy-prod`(节点连不上 Docker Hub,走 save/scp/load)。`just --list` 看全部 recipe。
- **发版节奏(用户定,长期有效)**:改动做完后**推送 github `main` 由你自动完成**(不用问)。**是否发版由用户决策**(等用户说「发版」)——但用户一旦说发版,后面**一条龙由你做完:版本号 bump + 打 `v*` tag + 触发 release CI + 部署 + 验证**,不用再等「部署」二次指令。补丁位递增(如 0.12.1→0.12.2,新功能也走补丁),minor 由用户拍板。**部署**:等 release CI 把 ACR 镜像推好后跑 `just deploy-prod X.Y.Z`(Bash 工具即 Git Bash,cygpath 在;SSH key 认证到 `root@mica.cloudcele.com` 免密),跑完验证 `/api/health` 报对版本 + 冒烟测本次改动。
- **踩过的坑见 `docs/lessons.md`(和本文件一起读)**:双表示红线、"不变量只写在客户端等于没写"、测试真空通过、web 通过≠桌面通过、图片解码 dispose 时序、round-trip 红线等。CLAUDE.md 写规则,那份写规则的来历——规则很容易被当成"大概是这个意思"绕过去。
- 键盘快捷键清单见 **`docs/shortcuts.md`**(权威)。加/改快捷键三处同步:`editor.dart` key handler(编辑器)+ `main.dart` `_appShortcuts`(应用级)+ `dialogs.dart` `_shortcutsSection`(设置面板)+ 该文档。

## 开发机与工具链(Windows 主力机)

- **当前主力机(2026-07-20 起)**:仓库在 `C:\data\codes\mica-will-laptop`。上一台主力机
  (仓库在 `D:\codes\mica`)硬件故障报废,已整体迁到本机;文档里再看到 `D:/codes/mica`
  一律按本路径理解。
- Flutter stable 装在 `C:\flutter`(已进用户级 PATH);Visual Studio **Build Tools** + Windows SDK
  已装,`flutter build windows` 可用——不需要 VS Community。Rust 工具链、Git 齐。
  Flutter app 在 `clients/mica_flutter/`,仓库根是 Rust workspace。
- **国内网络必须走镜像**:Flutter SDK 从 `storage.googleapis.com` 下是几分钟 60MB 的量级
  (1.8GB 要跑几小时),换官方中国镜像 `storage.flutter-io.cn` 后实测 122 秒 / 14.86 MB/s。
  已设用户级环境变量 `FLUTTER_STORAGE_BASE_URL=https://storage.flutter-io.cn` +
  `PUB_HOSTED_URL=https://pub.flutter-io.cn`,否则 `flutter pub get` 会卡在同一个问题上。
- **`just` 的 shebang recipe 绕过 `set windows-shell`**,里面用到 `cygpath`。跑
  `just deploy-prod` 前 `export PATH="/c/Program Files/Git/usr/bin:$PATH"`,或直接在 Git Bash 里跑。
- **强制重启/断电后 `.dart_tool/flutter_build` 缓存可能被截断** → frontend_server 报
  `RangeError`(离谱 offset)。`rm -rf clients/mica_flutter/.dart_tool/flutter_build` 即愈。
- **构建按标准来(不做热限流)**:上一台主力机报废前有过 GPU/整机硬卡死(发生在重型构建
  窗口内),但那是**设备自身故障**,不作为本机的约束——本机按主流做法正常构建:`flutter build
  windows` 全量该跑就跑,`cargo` 用默认并行度,不再为「功耗尖峰」人为设 `CARGO_BUILD_JOBS`
  上限或回避重活/绕道单编 vcxproj。能交 CI 就交 CI 仍是好习惯(省本地时间 + 多平台覆盖),但
  那是工程取舍,不是怕热。Docker Desktop 仍**非必要不启**,但理由与热无关:WSL2 冷启动要
  100s+,且 `docker info` 在启动中会**阻塞**而非快速失败——别用十秒超时去判"卡死",踩过一次
  白等 15 分钟。
- **runner C++(`windows/runner/*.cpp`)是 warning-as-error**:注释必须纯 ASCII(CJK 代码页撞
  C4819)、`_wgetenv` 已弃用要换 `GetEnvironmentVariableW`。

## 生产运维要点

节点是阿里云 `mica.cloudcele.com`,key 认证免密(主机名/registry 本来就在 `justfile` 里)。
容器名 **`mica-postgres-1`**(不是 `mica-postgres`)。详见 `docs/deploy.md` / `docs/release.md`。

- ⚠️ **`deploy-prod` 不做迁移前的数据库备份。** 它的顺序是 sync compose → pull/up api+web
  (**api 一起来就跑迁移**)→ 刷 backup sidecar → 健康检查;那个 sidecar 是**周期性导出器,
  且在 api 起来之后才刷**,当不了回滚点。**带数据改动的迁移必须自己先落还原点**:
  `docker exec mica-postgres-1 pg_dump -U mica -d mica | gzip > /data/mica/pre-<x>-<ts>.sql.gz`,
  再 `gzip -t` 验完整性 + `zcat | grep -c "^COPY public.<表>"` 确认目标表在内(库 22MB,几秒钟)。
- ⚠️ **SSH 连太密会被上游掐**:一个会话里连十几次之后开始清一色
  `Connection closed by <ip> port 22`。**不是节点干的**(fail2ban 没装,iptables 里也没规则),
  是云上游限流。**别猛敲重试,那只会续期**,等 ~7 分钟自然恢复。同期 443 也会偶发
  `schannel: failed to receive handshake`,重试即通。**验证脚本一次 ssh 里用 heredoc 跑完所有
  psql,别一条命令一个 ssh。**
- **分层生效**:服务端改动随 api 部署即生效;**MCP 代理层的改动在 `mica-cli` 二进制里**,
  用户不把 MCP 指向新版 mica-cli 并重连就还是旧行为。排查"我明明改了怎么没生效"先分清这层。
- **迁移是 `sqlx::migrate!` 编译期嵌入的**,新增迁移文件不触发 `mica-infra` 重编 →
  加迁移后 `touch crates/infra/src/db.rs` 强制重编,否则 `run_migrations` 还带旧集合。

## 协作约定(用户定,长期有效)

- **对话与项目文档用中文**(README 面向公众用英文)。
- **计划批准后连续执行到完成**,中途决策按推荐项走,不逐步请示;诚实报告做不到的部分。
- **能用子代理/workflow 提效就主动用**(用户显式允许)。但**"边改文件边攒清单"这类批量活
  必须单 agent 顺序跑完、不打断**:子代理的文件编辑是真实落盘,被打断时清单随之丢失,
  留下一堆悬空引用而重建信息已经没了(i18n 那次并行派三个、拒了两个,136 个 key 的翻译全丢)。
