# 双实现盘点与首页评估（2026-11-10）

> **状态：第 1–3 项已实施并经审核。第二轮实现及官方 CI／发布演练已完成，记录见第 6 节；5.4 已由用户确认按真实网络传输验收，4.71 MB 达标；Android 按用户要求豁免实机验证。**
>
> **原评估依据**：读代码和数行数；实施时再对照运行同一批输入。首页部分用示例数据单独渲染，不是线上截图。

## 1. 还有哪些双实现

按"砍掉后的收益 / 改动风险"排序。

| # | 双实现 | 现状 | 建议 |
| --- | --- | --- | --- |
| 1 | **Mermaid 图渲染** | 网页版用 2.5 MB 的 `mermaid.min.js`（`editor/mermaid_preview_web.dart`），桌面和手机用 Rust 渲染器（`mica_md::render`，经 `mermaid_preview_stub.dart`）。同一张图两端可能画得不一样 | 值得统一到 Rust。难点是网页 wasm 的体积预算（现在每个 wasm 限 256 KB gzip），要先量加进去多大 |
| 2 | **公式渲染** | 编辑器里用 Dart 的 `flutter_math_fork`；导出 HTML 用 Rust 的 `ratex-*`（`crates/md-core` 的 `render` 特性）。同一个公式编辑时和导出后可能不一致 | 中期统一到 Rust；先确认 ratex 对常用公式的覆盖 |
| 3 | **`crates/markdown` 兼容层** | 它的模块注释自己写着"新代码应直接依赖 `mica-md`"，但全仓还有约 190 处经 `mica_markdown::` 转手调用（`import_markdown` 69 处、`export_markdown` 35 处最多）。其中页标题、HTML 整页导出、MCP 文本编辑、LaTeX 检查是它自己的逻辑，不是转手 | 把纯转出的那部分调用改成直接依赖引擎，机械替换、风险低。**建议先做这一项** |
| 4 | **本地文档存储** | 桌面是 Rust + SQLite（`mica-core` 的 `store`）；网页是 Dart 手写的 IndexedDB 版（`cloud/web_idb_doc_store.dart`，约 630 行）。同一套"快照 + 本地待发日志 + 远端日志 + 游标"两份实现 | 收益大、改动也大。等 [GPUI 方向](../architecture/ui-gpui-evaluation.md) 定了再说 |
| 5 | **发版两条路** | GitHub Actions 的 `release.yml`（668 行）和本地脚本（`scripts/release*.sh`、`ship.sh`）各一套，都在维护、各有测试。0.14.4 走的是本地脚本 | 定一条为主，另一条只保留到"能应急" |
| 6 | **"本地模式"和"云端 + 离线缓存"** | 两套世界各有后端（`world/local_world_backend.dart` 966 行，`local/` 目录约 2,600 行）。云端模式已经能离线读写 | 产品决策：还要不要保留"无账号纯本地"这个用法 |
| 7 | **部署两份 compose** | 生产版 `docker-compose.yml` 413 行，单机快速版 `docker-compose.single.yml` 215 行 | 用途不同，建议保留 |
| 8 | **代码高亮** | 只有 Dart 一份（`editor/highlight.dart`，2,432 行）；导出的 HTML 没有高亮 | 不是双实现，是导出缺功能，另算 |

**消不掉的平台分叉**：17 个 `_web.dart` 文件、32 处 `kIsWeb` 判断，是 Flutter 一套代码跑网页和原生本身带来的，不迁框架就消不掉。

**核对过、不是重复的**：`deploy/restore-drill.sh` 和 `scripts/restore-drill.sh` 同名但一个是在节点上跑的本体，一个是把它传过去执行的包装。

### 前三项实施记录（2026-11-11）

- **Mermaid**：网页版旧脚本与 Rust merman 均渲染了中文流程图、时序图、类图和架构图；其中同一张中文流程图的 `viewBox` 分别是 `0 0 250.03125 63` 和 `0 0 250 70`。现在网页经 Web Worker、桌面与手机经 FFI、HTML/PDF 导出原生调用同一份 merman。编辑器两端共用主题映射、CSS 内联、内置中文字体回退和有尺寸上限的 SVG 光栅化；失败时仍显示代码。
- **公式**：旧编辑器和 RaTeX 对照了上标、分式、根式、求和、积分、矩阵、对齐环境、中文文本和自定义颜色；9 个有效输入均可排版，未知命令和缺参分式均拒绝。编辑器改用导出的 RaTeX，分别按行内、块级样式生成 SVG；行内位置用 Rust 返回的基线。无法渲染时保留源码。
- **兼容层**：解析、序列化、HTML 片段、属性和模型的调用改用 `mica_md`。页标题、整页 HTML、文本编辑和 LaTeX 检查留在 `mica-markdown`。（审核后又删去了门面里剩下的引擎转出，见下方审核记录。）

生产设置下分别对 wasm 和 JS 执行 `gzip -n -9` 后相加：Mermaid **1,895,322 B**（旧 JS **743,567 B**），公式 **976,731 B**。如果把它们塞入 Markdown 启动包，会直接超过 256 KiB 门槛；经服务器渲染则需要上传源码，也会丢掉已载入引擎后的离线预览能力。两份渲染器各在首次出现对应内容时下载，门槛分别为 2 MiB、1 MiB；Markdown 和文档引擎的 256 KiB 门槛保持原样。含 Mermaid 的页面首次加载时要多下载约 1.10 MiB；普通页面不下载渲染器。

### 前三项审核记录（2026-11-11）

审核对象：`96dbb35e`（兼容层）、`386dfc8f`（Mermaid 与公式）。结论：三项都达到目标，没有阻止发版的问题；
审核中发现的一个缺陷和两个收尾项已修复（下面的修复提交）。

**审核时实际跑过的**（一台云端 Linux 机器）：

- Rust：格式检查、clippy（警告即失败）、工作区测试全部通过。6 个 AI 流式测试需要真实 Postgres，跳过；该文件 10 月 26 日后未改，与本次无关。
- 三个网页引擎重新构建，压缩后体积与实施记录一致：公式 976,890 B、Mermaid 1,895,131 B，启动包两个引擎各约 250 KB，均在预算内。
- Flutter：静态检查无错误，全部测试通过，其中包括桌面端走原生接口的公式与 Mermaid 用例。
- Chromium：渲染回归脚本和网页单元测试通过，截图中行内公式、块级公式、Mermaid 和中文标签都正确。

**发现并已修复**：

1. **渲染器没能运行时，排队中的预览被永久标为失败。** 网页渲染器因某条源码崩溃、或渲染器文件下载失败时，
   同一时刻在排队的所有公式和图都被记为"渲染不了"，之后一直显示源码，直到切换主题或刷新。
   修复：worker 区分"源码渲染不了"和"渲染器没能运行"；只有导致崩溃的那一条记为失败，其余暂停后重试
   （1 秒起、翻倍、最长 30 秒）。修复时还发现，浏览器会在同一个 worker 里记住失败的模块导入，
   在原 worker 里重试永远失败，所以加载失败时 worker 会自行关闭、换新 worker 重试。
   新增测试：流水线的重试与退避（VM）、worker 回复协议（浏览器，用脚本化的假 worker）、
   以及端到端"先拦截渲染器文件、再放行，预览自动出现"（真实 worker 与真实渲染器）。
2. **兼容层还留着引擎转出。** `mica-markdown` 仍转出 `import_markdown` 等引擎接口，`app-core` 又整体转出它，
   旧路径实际还在；保护它的测试以"有看不到的调用方"为由，但该 crate 不对外发布，仓库内已无调用方。
   修复：删去全部转出和无人调用的包装函数；`app-core` 只转出 `Block`、`DocumentSnapshotPayload`；
   `block_index`、`is_descendant` 移入唯一使用它们的 `app-core`；删除 `facade_surface.rs`。
3. **部署后检查脚本没覆盖新文件。** `web-entry-cache-check.sh` 现在检查 worker 与两个渲染器的状态码、
   MIME 和缓存头，缺失的渲染器文件返回 404，旧 `mermaid.min.js` 返回 404，并补了对应的回归用例。
   注意：对线上 0.14.4 运行会失败（那时还没有渲染器文件），下一个版本部署后才会通过。

**取舍，记录在案**：网页首次遇到公式要多下载约 1 MB（以前公式库在主包里），遇到 Mermaid 从 743 KB 增至 1.9 MB；
普通页面不下载。两个渲染器没有离线缓存，网页离线时首次遇到公式或图会先显示源码，联网后自动出现预览。
主包因去掉公式库变小了多少，未测。

**与本次无关、但审核中发现的**：

- 剪贴板浏览器回归脚本有一项失败（复制表格单元格时行内代码丢格式）。在两个提交之前的版本上复测同样失败，是之前就有的问题。
  **已修复（2026-11-11）**：真正的原因不是复制，而是单元格编辑器里的 Ctrl+A。网页上它由浏览器在隐藏输入框里完成，
  再异步同步回 Flutter；紧接着打字之后按下时，同步可能晚于随后的 Ctrl+B，于是加粗作用在空选区上。
  现在单元格编辑器自己处理 Ctrl+A，不再依赖这次往返；脚本连跑三次全部通过，并补了单元测试。
- main 分支的 CI 自 10 月 20 日起每次都是"已取消"，没有机器在跑，提交到 main 实际没有自动门禁。
  **处理（2026-11-11）**：自建 runner 已停用，门禁改为本地 CI（`just ci`），已写入 `AGENTS.md` 和 `docs/ops/ci-runners.md`。

## 2. 首页要不要改

首页现在有三块：

| 块 | 别处有没有 |
| --- | --- |
| "新建页面"大卡片 | 有：侧栏的加号、Ctrl+N |
| "最近编辑"（跨工作空间） | 只有搜索框的预输入列表有，而且那个只限当前工作空间。**这是首页唯一独有的东西** |
| "目录" | 有：侧栏页树 |

**为什么用不上**：应用启动时会恢复上次打开的标签页（`doc_tab_persistence.dart`），所以只有把页面全部关掉才会看到首页。

**建议：精简，不要重新设计。**

- **A（推荐）**：保留"最近编辑"，去掉"目录"，把新建卡片缩成一个按钮。首页变成轻量的"继续上次的工作"页。
- **B**：去掉首页，关掉所有页面后直接打开最近编辑的那篇；"最近编辑"只留在搜索框里。最省，但失去跨工作空间的入口。
- **C**：不动。

**顺带看到的一处**：示例渲染里，目录行右侧的"12 个页面"被折成了两行。用的是我编的示例文字，线上是否也这样没有确认。

## 3. 第二轮评估（2026-11-11）

前三项完成后再查一轮：超大单元、重复实现、无用代码与依赖、手写与生成重复、首次加载体积。只读和实测，没有改代码。

| # | 发现 | 依据 |
| --- | --- | --- |
| 1 | **本地 CI 只覆盖了原 GitHub CI 的一部分** | GitHub CI 还跑 4 个浏览器端到端脚本、结构检查和约 10 个脚本自测（清单见 5.2），`local-ci.sh` 都不跑；runner 停后它们没有任何地方在跑。剪贴板那个问题就是这样没被发现 |
| 2 | **网页版首次打开下载 24.9 MB，一半是两个中文字体** | 实测首次加载：`NotoSansSC-Regular.otf` 8.48 MB、`DroidSansFallback.ttf` 4.03 MB，登录页就下载；线上按 `application/octet-stream` 返回、不在压缩列表里，原样传输。对比：主程序 1.86 MB、渲染引擎 2.32 MB（压缩后）。同源镜像 `web/gfonts/notosanssc/` 已有全部思源黑体按字切片（101 个、2.6 MB），用到才加载 |
| 3 | **桌面版和安卓版也带着这 12.5 MB 字体** | 那里只是最后的兜底，系统有中文字体；安卓安装包 45–53 MB、Windows 安装包 41 MB 都含这两个字体 |
| 4 | **应用外壳是最大的维护负担** | `main.dart` 库合计 17,009 行，唯一超过 8,000 行硬上限的单元（已登记）；其中 `main_shell.dart` 5,277 行，一个状态类、164 个私有方法、79 处 `setState` |
| 5 | **路线图文档大部分是元叙述** | `roadmap.md` 181 行，未完成的只有 3–4 条 |
| 6 | **零碎** | `mica-core` 声明了没用到的 `serde`（`cargo-machete`）；Flutter 静态检查约 80 条提示 |

**查过、没问题的**：前端接口模型是有意手写，服务端与前端都对照 `contracts/` 里的同一组样例测试；
两份约 1,850 行的导入代码一份规划、一份执行（且后者一半是测试），不算重复；前端没有无用依赖；
所有脚本都有引用；网页构建里两个 13 MB 字体目录一个内置、一个是按需切片，不是重复（但见第 2 条）。

## 4. 用户决定（2026-11-11）

| 事项 | 决定 |
| --- | --- |
| 第二轮评估的 6 项新发现 | **都要做**，见第 5 节清单 |
| 两条发版路径（第 5 项） | **两条都保留**。GitHub Actions 从自建 runner 切回 GitHub 官方 runner；Actions 额度用完时直接走本地流程 |
| 网页与桌面两套本地存储（第 4 项） | 等 [GPUI 方向](../architecture/ui-gpui-evaluation.md) 定了再说 |
| 本地模式（第 6 项） | **保留** |
| 首页 | **先不动** |

## 5. 待办清单（交给他人实施，完成后回来评审）

> 每一项做完：在本文对应条目打勾，写上提交号和实测数字；推送 `main` 前跑完本地 CI（`just ci`），
> 改到网页行为的再跑对应的浏览器端到端脚本。不要为过检查而加等待、跳过测试或放宽预算；
> 编辑器、渲染、主界面三个库的行数棘轮只许减不许增（`scripts/file-size-budget.txt`）。

### 5.1 GitHub Actions 切回官方 runner（第 1 节第 5 项的决定）

- [x] 所有工作流的 `runs-on` 从 `[self-hosted, Linux]` / `[self-hosted, Windows]` 改回 `ubuntu-latest` / `windows-latest`
  （各处都留着注释掉的原值）。触发方式保持手动（`workflow_dispatch`），不要改回推送即触发，以免自动消耗额度。
- [x] **先确认能连到生产节点**：`deploy.yml`、`dr-drill.yml`、`prod-diagnostics.yml` 要 SSH 到生产节点。
  确认 GitHub 官方 runner 能否直接连上（生产 SSH 是否对公网开放）。连不上就保留这三个用本地流程
  （`just deploy-prod` 等），并在文件顶部注释说明原因；不要为此在仓库里放任何凭据或内网地址。
- [x] `runner-bootstrap.yml`、`runner-probe.yml` 只为自建 runner 存在：删除，或注明"仅在重新启用自建 runner 时使用"。
- [x] 去掉只对自建机器有意义的步骤或绕法（例如复用机器上预装 Flutter 的逻辑、"别碰 ~/.ssh"的隔离），
  确认官方 runner 上不需要；需要保留的，确认在官方 runner 上同样成立。
- [x] 手动触发一次 `CI`（scope=full），全部通过；再触发一次 `Release dry run`，通过。把两次运行的链接写进本文。
- [x] 更新 `AGENTS.md`（现在写的是"自建 runner 已停，本地 CI 是唯一门禁"）和 `docs/ops/ci-runners.md`：
  改为"GitHub Actions 用官方 runner、手动触发；额度用完时走本地流程（`just ci`、`just release` 等）"。
  `docs/ops/release.md` 里两条发版路径都要写清楚，并说明何时用哪条。

### 5.2 本地 CI 补齐原先只在 GitHub CI 里跑的检查（第二轮评估第 1 项）

`.github/workflows/ci.yml` 跑而 `scripts/local-ci.sh` 不跑的：

- [x] 结构检查：`scripts/test-structure-gate.sh`、`scripts/check-structure-ci.sh`
- [x] 脚本自测：`test-md-wasm-build.sh`、`test-web-entry-cache-check.sh`、`test-release-artifacts.sh`、
  `test-release-orchestration.sh`、`test-release-version.sh`、`test-ci-stacks.sh`、`test-verify-prod.sh`、
  `test-compose-api-env.py`
- [x] 4 个浏览器端到端脚本：`e2e/rich_preview_e2e.mjs`、`e2e/clipboard_e2e.mjs`、`e2e/ui_experience_e2e.mjs`、
  `e2e/sidebar_rename_e2e.mjs`。它们不需要后端：照 `ci.yml` 第 560–615 行的做法构建 `e2e/web_harness_entry.dart`、
  起静态文件服务即可。建议做成 `just ci e2e` 这一部分，`just ci`（all）也包含它。
- [x] 浏览器来源：仓库锁定的 Playwright 版本要配套下载自己的浏览器；在已有 Chromium、不能下载的机器上要能用
  `CHROME_EXECUTABLE` 指定浏览器（现在脚本只回退到系统 Chrome）。
- [x] 明确不纳入本地 CI 的（需要真实服务栈或 Docker，如 `web_e2e.mjs`、`upload_length_e2e.mjs`、容器栈）
  继续列在脚本结尾的"未覆盖"清单里。
- **验收**：`just ci` 全部通过；故意改坏一处（例如让剪贴板复制丢格式）能让 `just ci` 失败。

### 5.3 字体压缩（第二轮评估第 2 项 a）

- [x] `deploy/nginx.conf` 与 `deploy/nginx.dev.conf`：给 `.otf`、`.ttf`（以及 `.woff2`）配上正确的 MIME 类型；
  `.otf`、`.ttf` 加入 `gzip_types`（`.woff2` 本身已压缩，不要再压）。
- [x] `scripts/web-entry-cache-check.sh` 加一条：字体以正确的 MIME 类型返回，并在其自测脚本里补用例。
- **验收**：本地容器或部署后实测，`DroidSansFallback.ttf` 传输约 2.1 MB（原 4.0 MB），
  `NotoSansSC-Regular.otf` 约 7.3 MB（原 8.5 MB）。

### 5.4 网页版不再内置两个中文字体（第二轮评估第 2 项 b）

- [x] 网页构建不再打包 `NotoSansSC-Regular.otf`、`DroidSansFallback.ttf`，中文改由引擎从同源镜像
  `web/gfonts/notosanssc/` 按需加载（镜像已有全部 101 个切片，共 2.6 MB）。桌面和手机不受影响（见 5.5）。
- [x] `lib/cjk_fonts.dart` 网页分支、`lib/editor/svg_raster.dart`（Mermaid 中文标签目前指定内置字体名）
  相应调整，保证 Mermaid 图里的中文仍然不是方框（`rich_preview_e2e.mjs` 已有"不同汉字画出不同字形"的检查）。
- [x] 评估并记录首次显示中文时的闪烁（方框 → 正确字形）是否可接受；离线时未加载过的字如何显示。
- **验收**：首次打开登录页的下载总量从 24.9 MB 降到约 12 MB 以下（用同样的方法实测：记录首次加载的全部响应大小）；
  `rich_preview_e2e.mjs`、`clipboard_e2e.mjs` 通过；编辑器里中英混排、表格、公式、Mermaid 的截图无方框。

### 5.5 桌面版和安卓版的内置字体（第二轮评估第 3 项）

**补充验收范围**：用户允许跳过 Android 实机验证；保留 Android 字体。Windows 和 Linux 已完成字体对照及截图，详见 6.2。

- [x] 实测 Windows、Linux 包中字体及移除后的中、日、韩文、生僻字显示；Android 按用户补充要求豁免实机验证，保留字体且不声称重建验证。
- [x] 能覆盖的平台去掉内置字体（只保留在确实需要兜底的平台）；不能覆盖的保留并在本文写明原因。
- **验收**：本文记录各平台安装包变化前后的大小，以及真机或模拟器上的截图结论。

### 5.6 拆分应用外壳（第二轮评估第 4 项）

- [x] 从 `lib/main_shell.dart`（5,277 行、一个状态类、164 个私有方法）继续拆出独立控制器，
  沿用现有 `views/`、`auth/`、`sync/` 控制器的做法；每次一个主题、几百行，单独提交，行为不变。
- [x] 每次拆完收紧 `scripts/file-size-budget.txt` 里 `main.dart` 的基线。
- **验收**：本轮把 `main.dart` 库从 17,009 行降到 15,000 行以下；全部测试通过，主流程（登录、切换工作区、
  打开页面、本地模式、离线编辑）手动或端到端验证无回归。

### 5.7 精简路线图（第二轮评估第 5 项）

- [x] `docs/plans/roadmap.md` 精简到只剩未完成事项及其状态（约 40 行），"接下来最该做的"一节里关于清单本身反复过时的历史
  移到 `roadmap-done.md` 或删除；保留文件顶部的维护规矩。

### 5.8 零碎清理（第二轮评估第 6 项）

- [x] 删除 `crates/mica-core/Cargo.toml` 里未使用的 `serde` 依赖（`cargo-machete` 报告；只用到 `serde_json`）。
- [x] 清理 Flutter 静态检查的提示（约 80 条，其中 27 条 `library_private_types_in_public_api`），只改提示所指的写法，不改行为。

### 不在本轮

- 网页与桌面两套本地存储：等 GPUI 方向定。
- 本地模式：保留，不动。
- 首页：先不动。


## 6. 第二轮实施与验收记录（2026-11-11）

尚未发版、未部署。实施提交：字体与渲染 `cd03ebed`；外壳与偏好控制器 `ba27965d`；CI 与项目规则 `36ca950a`；静态清理、serde 与路线图 `dcea64d8`。追加 hosted runner 工具前提修复：`eda11774`。

官方最终运行：[CI(scope=full)，全部通过](https://github.com/weironz/mica/actions/runs/42988594674)、[Release dry-run，全部通过](https://github.com/weironz/mica/actions/runs/42985698738)、[生产只读诊断，通过](https://github.com/weironz/mica/actions/runs/42985702830)。发版演练未发布 release、未部署。

恢复托管执行后修复了三个旧 CI 前提：`eda11774` 安装脚本自测实际需要的 just；`6fa636e8` 用标准迁移入口登记账本，避免凭据轮换测试再次建表；`1b01fb3d` 删除对已在 `8291c551` 退役的一次性审计测试的失效调用。没有降低回执下限或跳过现存测试。

### 6.1 已实现的范围

- 工作流改为官方 runner，保留手动触发；恢复 macOS CLI 构建。删除自建 runner 的 bootstrap/probe 工作流、预装 Flutter 复用及无人使用的 bootstrap 脚本。临时 SSH 和 Docker 凭据隔离仍适用于官方 runner，保留。
- 本地 CI 增加结构检查、脚本自测和四个独立浏览器回归；`just ci e2e` 可单独运行。Playwright 使用锁文件，支持显式 `CHROME_EXECUTABLE`。真实服务栈、容器、MCP inspector 和 Windows 集成仍列为独立检查。
- nginx 两份配置添加字体 MIME、TTF/OTF gzip（level 6），WOFF2 不重复压缩；入口契约检查同时拒绝残留的两个完整中文字体。
- Web 改用引擎按需字体。补齐引擎可能请求的 JP/KR/TC/HK 镜像，避免去掉完整字体后出现 404。新增大列表自测：原镜像脚本在 `pipefail` 下用 `echo | grep -q`，列表变大后 SIGPIPE 会把有效文件误判为过期；现改为完整读取。
- SVG 缓存在系统字体到达时作废，并拒绝字体到达前的在途结果。登录分段按钮重建字体固有宽度缓存，回归检查实际 RenderParagraph 的行数和裁切，避免以按钮外框尺寸冒充文字尺寸。
- 外壳库 **17,009 → 14,606 行**，`main_shell.dart` 为 5,191 行。六类对话框成为独立库；偏好读取、保存、云同步和定时器移入 `ShellPreferencesController`。延迟写入捕获数据和会话归属，过期云回复不能覆盖下一会话。没有将 part 拆分冒充库拆分。
- 路线图 **181 → 40 行**，保留实际未完成项；历史移入归档。删除 mica-core 未用 serde，更新两个锁文件；静态检查从最初 lib 的 77 条提示降至全项目无提示（最终检查另附）。渲染器原有公开接口使用的布局类型正名为 `NodeLayout`，表格内部字段继续私有。

### 6.2 体积与平台实测

| 项目 | 前 | 后 | 口径与结论 |
| --- | ---: | ---: | --- |
| DroidSansFallback.ttf | 4,033,420 B | 2,146,464 B | 本地 nginx 容器实际 gzip 响应体 |
| NotoSansSC-Regular.otf | 8,482,020 B | 7,341,890 B | 同上；两者 MIME 和 Content-Encoding 正确 |
| Web 登录页实际传输 | 17,591,304 B | 4,713,883 B | 同机同 SDK、相同冷上下文；基线界面来自 32346b2a，两边使用同批 Rust 引擎产物；Chrome CDP 全部网络响应，含响应头 |
| Web 解压后响应体 | 24,889,467 B | 12,776,275 B | 这才对应原文约 24.9 MB 的口径；保留原始口径供比较；用户已确认 12 MB 门槛约束真实网络传输，不约束解压体积 |
| Windows 安装包 | 26,091,033 B | 18,473,565 B | 同一 Windows 主机、同一工具链和当前源码分别构建；移除字体后减少 7,617,468 B；含登录标签回归修复的少量代码差异 |
| Linux deb | 24,326,988 B | 23,913,560 B | 前为已发布 0.14.4，后为本次本机构建，构建环境和代码有差异；保留字体，字体资源净减少 0 B，不把整体差值归因于字体 |
| Android | arm64 53,294,048 B；armv7 45,715,354 B | 未重建 | 用户明确允许跳过设备验证，保留字体，不声称真机通过 |

Web 原始逐响应记录：[同机基线](assets/redundancy-2026-11-11/web-baseline.json)、[最终构建](assets/redundancy-2026-11-11/web-after.json)。用户于 2026-11-11 明确确认：**12 MB 门槛约束真实网络传输，当前 4.71 MB 已达标**。解压体积仍为 12.78 MB，保留该数据，不将两种口径混用。当前线上未部署上述修改。

Windows 在用户提供的设备上对照了有/无内置字体，并在最终实际不含两字体的包上再次运行字体探针；常用中、日、韩文及 `𠮷`、`𠀀` 样本、Mermaid 标签正常。所选系统字体也在 [Microsoft Windows 11 字体清单](https://learn.microsoft.com/en-us/typography/fonts/windows_11_font_list) 中。截图：[移除前](assets/redundancy-2026-11-11/windows-before.png)、[最终移除后](assets/redundancy-2026-11-11/windows-after.png)。这不是对任意 Unicode 字符完整覆盖的承诺。

Linux 发行版不能保证装有 CJK 系统字体。仅保留拉丁系统字体时，[去掉内置字体](assets/redundancy-2026-11-11/linux-without-cjk-fonts.png) 会让常用中文、日文缺字，因此保留兜底。[正常系统截图](assets/redundancy-2026-11-11/linux-normal.png) 中常用中、日、韩文正常，但 `𠀀` 仍缺字；内置字体也没有覆盖所有扩展区。像素不同不等于每个字都有字形，不能仅以两个栅格不同宣称完整覆盖。

Web 的字体按需加载会在冷缓存、慢网络时短暂缺字；未加载过的字符离线不能保证显示。当前标签页已加载字形保留，联网后的恢复还受引擎字体重试机制影响；必要时刷新，不承诺离线全字库。[登录截图](assets/redundancy-2026-11-11/web-login.png) 与浏览器回归验证了联网稳定后的显示。

### 6.3 门禁与最终验收

5.4 字体删除和体积验收均已完成。验收口径经用户明确确认为真实网络传输，4.71 MB 低于 12 MB；解压后的 12.78 MB 作为独立观测值保留。

- 完整 `just ci` 已通过，故障注入恢复后再次全套通过：Rust 工作区回执 **1,191**，带 store 特性的 mica-core 回执 **221**，Flutter 单元/widget 回执 **2,096**，浏览器专用套件及四个 Node 浏览器脚本通过；这些是各次执行数，不宣称互不重叠的唯一测试总数。
- 故障注入：临时把 ClipboardItem 的 HTML 内容改成纯文本，完整 `just ci` 在 `clipboard_e2e.mjs` 的 Gemini 行内数学格式断言失败，退出 **1**；源码已逐字节恢复。没有放宽预算或新增跳过。
- Windows 字体探针、安装包构建已通过；另做的完整应用首帧冒烟没有取得原生首帧回执。诊断已显示启动完成窗口、引擎、字体初始化并执行 Dart 首帧，但远程会话创建交换链失败（DXGI `0x887A0022`，EGL Context Lost）；不把 GPU 呈现失败伪装成启动通过。官方 Windows runner 的实际安装包安装、首帧回执和首帧后存活 10 秒已通过，见上述 Release dry-run 的 Windows 作业。
- 官方 runner 的生产 SSH 已由只读诊断实际确认；`CI(scope=full)` 和 `Release dry-run` 全部通过，5.1 完成。
- 5.6 的验证组合为：真实服务栈 Web E2E、四个浏览器交互回归、真实 FFI 指纹检查的本地/离线模块测试，以及 Windows 实际安装启动冒烟，均通过；不声称逐一人工操作了所有桌面流程。

## 7. 第二轮审核记录（2026-11-11）

审核对象：`cd03ebed` `ba27965d` `36ca950a` `dcea64d8` `eda11774` `6fa636e8` `1b01fb3d` 及其文档提交。
**结论：5.1–5.8 全部达到验收标准（5.4 的口径与 5.5 的安卓范围按用户确认调整），没有需要返工的问题。**

**审核时独立复核的**（云端 Linux 机器）：

- 完整 `just ci` 通过（含数据库测试与新增的浏览器端到端部分）：Rust 1,191、带 store 的 mica-core 221、Flutter 2,096，
  浏览器专用套件全部通过；四个浏览器脚本（公式与 Mermaid 预览、剪贴板、界面体验、侧栏重命名）全部断言通过。
- 官方 runner 的三次运行确认成功：CI（scope=full，`1b01fb3d`）、Release dry-run（`05595908`）、生产只读诊断。
- 网页构建里已没有两个中文字体（`FontManifest.json` 只剩 MaterialIcons、RobotoMono、Roboto）；
  首次打开实测解压后 12.8 MB（原 24.9 MB），中文改为按需加载的切片。
- 外壳库 14,606 行，预算已收紧；拆出的六个对话框是独立库（各自 import，不是 `part of`）。
- 读过的关键改动：nginx 新增的字体 location 没有丢失服务器级响应头（配置里本来就没有服务器级 `add_header`）；
  JS 互操作的类型判断改为 `isA<>`，是正确性修复而非仅消除提示；偏好控制器把推送数据和账号在排队时捕获，
  修掉了旧计时器可能把上一账号设置推给下一账号的问题。

**不阻塞、但建议知道的**：

1. **macOS CLI 构建恢复了**：GitHub 对 macOS 分钟按 10 倍计费，每次手动 Release 消耗额度更快。若额度紧张，可改为可选输入。
2. **官方 runner 构建的 APK 每次用新的调试签名**（文档已写明）：它装不上、也升级不了之前的 APK。需要可覆盖安装时继续在同一台本地机器构建，或把签名密钥存为仓库机密。
3. **字体镜像从 11.9 MB 增到 21.8 MB**（补齐日、韩、繁体切片，避免引擎请求 404），永久进入 git 历史，也增大网页镜像；用户只下载用到的切片。
4. **Release dry-run 跑在最后两个 CI 修复之前**：那两个只改测试脚本，不影响发版产物；下次发版前照常再跑一次即可。
5. **设置推送沿用排队时的令牌**：令牌恰好在那 1 秒内刷新时，这一次推送会失败（记日志，下次修改时会重推），影响可以忽略。
6. **Linux、安卓、macOS 启动时先加载两个中文字体再显示界面**：原先引擎启动时也要注册这两个字体，预计没有变慢，但未实测；方便时看一下安卓冷启动时间。
7. **尚未发版部署**：`web-entry-cache-check.sh` 现在会要求字体类型正确、旧的完整字体不存在，对线上 0.14.4 运行会失败，部署新版本后才会通过。
