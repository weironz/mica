# 双实现盘点与首页评估（2026-11-10）

> **状态：前两轮已实施并审核；第三轮第 8 节实施中，进度和证据见第 9 节。涉及设备、签名/发布和上游修复的部分不提前标为通过。**
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

## 8. 第三轮待办清单（2026-11-11 用户确定：全部要做；交给他人实施，完成后评审）

> 规则同第 5 节：每项做完打勾、写提交号和实测数字；推送 `main` 前跑完本地 CI（`just ci`）或手动触发官方 CI；
> 不加等待、不跳过测试、不放宽预算；三个大库的行数棘轮只许减不许增。
> 标"需用户"的项要用户本人操作或拍板，实施者准备好材料后交给用户；标"需设备"的项要真机或对应浏览器。
> 依据：第三轮检查时实测（生产响应头、`cargo audit`、`flutter pub outdated`），以及第 7 节审核中的非阻塞提示。

### 8.1 安全

- [x] **生产安全响应头**（`deploy/nginx.conf`、`deploy/nginx.dev.conf`）：现在首页和 `/api/` 都没有任何安全响应头，
  且响应里带 `Server: nginx/1.31.2`。加上 `Strict-Transport-Security`、`X-Content-Type-Options: nosniff`、
  防嵌入（`X-Frame-Options` 或 CSP `frame-ancestors`）、`Referrer-Policy`，并设 `server_tokens off`。
  注意 nginx 的 `add_header` 写在 location 里会覆盖上层的，要确认每个 location 都带上（用 `always` 让错误页也带）。
  分享页 `/s/` 已有自己的 CSP（`SAFE_HTML_CSP`），不要被覆盖。
- [x] **应用的内容安全策略（CSP）**：为 Flutter 网页单独写一份（需要放行 `wasm-unsafe-eval`、同源 worker、
  `blob:`/`data:` 图片、WebSocket 到同源 `/ws/` 等），先用 `Content-Security-Policy-Report-Only` 观察一轮再切正式。
- [x] `scripts/web-entry-cache-check.sh` 加检查：上述响应头存在、`Server` 不带版本；自测脚本补用例。
- **验收**：部署后 `curl -I https://mica.cloudcele.com/` 与 `/api/ready` 能看到这些头；四个浏览器端到端脚本和
  真实服务栈 Web E2E 通过；浏览器控制台无 CSP 违规。
- [x] **`ctutils` 0.4.2 → ≥ 0.4.4**（RUSTSEC-2026-0385，比较非恒定时间；经 `hmac`/`hkdf` 进入服务端）：
  `cargo update -p ctutils`，根锁文件。
- [x] **SQLite 漏洞**（`libsqlite3-sys` 0.35，RUSTSEC-2026-0459）：`crates/mica-core` 的 `rusqlite` 现为 0.37，
  升到带 `libsqlite3-sys` ≥ 0.39 的版本，根锁文件和 `clients/mica_flutter/rust/Cargo.lock` 都要更新；
  验证本地模式、离线编辑、数据库迁移（`cargo test -p mica-core --features store` 与 FFI 指纹检查）。
- [x] **`rmcp` 0.9.1**（RUSTSEC-2026-0462）：公告针对 Streamable HTTP **客户端**，我们只用 stdio 服务端，不受影响。
  在 `crates/mcp-server/Cargo.toml` 旁注明不受影响的理由；大版本升级（→ 3.x）评估工作量，单独做。
- [x] **把 `cargo audit` 纳入本地 CI 和官方 CI**：两个锁文件都扫；已知无修复版本的"无人维护"公告
  （`cgmath`、`paste`、`smallstr`、`ttf-parser`、`unsafe-libyaml`、`adler`、`pin-utils`）用忽略清单逐条写明原因，
  新出现的漏洞要让门禁失败。
- **验收**：`cargo audit` 对两个锁文件除忽略清单外零告警。

### 8.2 依赖升级

- [ ] Flutter 依赖分批升级：`file_picker` 8→13、`tray_manager` 0.5→0.8、`window_manager` 0.5→0.6、
  `screen_retriever` 0.2→0.3、`pasteboard` 0.5→0.6、`xml` 6→7、`flutter_svg`/`web`/`crypto` 小版本；
  `flutter_rust_bridge` 2.12→2.13 与 Rust 端、生成代码一起升。升级前先读 `pubspec.yaml` 里的锁定理由，
  有理由的保留或在本文说明为何解除。
- **验收**：每批单独提交；`just ci` 通过；Windows 桌面冒烟（窗口大小记忆、托盘、文件选择、剪贴板粘贴图片）通过；
  Linux .deb 启动正常。

### 8.3 性能与体积

- [x] **网页按需加载不常用模块**：用 Dart 延迟导入（`deferred as`）拆出 AI 对话框、版本历史、关系图、导入等不在首屏的模块。
  **验收**：首次打开登录页时 `main.dart.js` 体积下降（写明前后字节数），打开这些功能时首次加载正常，浏览器端到端脚本通过。
- [x] **评估 Flutter wasm 构建（skwasm）**：需要 COOP/COEP 跨域隔离响应头，并核对 Safari/Firefox 支持与对现有 worker、
  wasm 引擎的影响。只出评估结论和实测数字，不直接切换。
- [ ] **安卓冷启动实测**（需设备）：启动前加载两个中文字体是否拖慢首帧；有明显影响再改为首帧后加载。

### 8.4 代码结构

- [x] 继续拆分外壳：`main_shell.dart`（5,191 行）、`main_workspace_view.dart`（3,019 行），方法同 5.6。
  **验收**：`main.dart` 库从 14,606 降到 12,000 行以下，预算收紧，主流程无回归。
- [x] 渲染库 `render.dart`（5,733 行）：按路线图在下次新增块渲染器时抽出渲染器上下文接口并拆库；本轮若没有新增渲染器，
  先出接口设计草案写进本文。编辑器库（7,060 行）找一两个自成一体的部分拆出，收紧预算。

### 8.5 工程与发版

- [x] `release.yml` 的 macOS CLI 构建改为可选输入（默认关），GitHub 对 macOS 按 10 倍计费。
- [ ] 安卓签名二选一（**需用户**拍板）：把签名密钥存为仓库机密，让官方 runner 构建的 APK 能覆盖安装；或在发版文档约定
  APK 只从本地固定机器构建发布。
- [x] **发版产物验证（canary）**：在干净目录安装刚发布的版本（Windows 安装包、Linux CLI、.deb），校验能启动、
  `/api/health` 与 `--version` 版本一致、固定入口文件契约；串进发版流程（路线图待办）。
- [ ] yrs 模糊测试：跟进上游 #415，剩余 panic 修复后启用 4 条被跳过的 property 测试（路线图待办，依赖上游）。
- [x] 字体切片约 10 MB 进入 git 历史：评估是否改用 Git LFS，写结论；不改也要写明理由。
- [ ] 下次发版前重跑一次 `Release dry-run`；发版部署后运行 `web-entry-cache-check.sh`，确认字体与新响应头检查通过。

### 8.6 功能待办

- [x] **导入完成发邮件**：大批量导入在后台完成后给用户发邮件（复用 `mail.rs` 的邮件底座），含成功/失败计数与入口链接；
  用户可在设置里关闭。验收：真实服务栈下导入完成后收到邮件（测试邮箱或本地邮件捕获）。
- [ ] **`mica-md` 对外发布**（**需用户**）：按 [S5 发布手册](../single-source/stages/S5-sdk-release.md)准备好版本号、
  变更说明和发布前检查结果，由用户本人执行 crates.io / npm 发布。
- [ ] **issue #170**（点击一段时间后光标放不上去）：跟进报告人；拿到复现条件后定位修复，否则按项目惯例关闭并说明。

### 8.7 GPUI 方向的剩余验证（需设备，见 [评估文档](../architecture/ui-gpui-evaluation.md) 第 8 节）

- [ ] Windows 原生窗口下用真实中文输入法试 gpui-kit 与 Native Shell 的 Windows 后端（含是否重复上屏、候选窗位置）。
- [ ] 安卓真机上跑 Native Shell 的 `RichEditor` 示例：组字、长按选词、把手、系统菜单。
- [ ] Safari、Firefox 上重复评估文档 8.1 的 Web 测试。
- **验收**：结果写回评估文档第 8 节；四道关卡状态更新。网页与桌面两套本地存储的去留在此之后再定。

### 8.8 补充评估（只出结论，不直接改）

- [x] 服务端数据库查询性能：慢查询日志、常用接口的查询计划、索引覆盖；列出值得优化的前几项。
- [x] 无障碍：读屏、键盘可达性、对比度，按 Flutter 语义树检查主流程。
- [x] 多语言文案完整性：`app_zh.arb` 与 `app_en.arb` 键是否一致、是否有硬编码文案。
- [x] 移动端网页整体交互：在手机尺寸下走一遍主流程（登录、侧栏、编辑、选择、表格、设置），列出问题。

### 不在本轮

- 本地模式：保留。首页：不动。


## 9. 第三轮实施记录（2026-11-11，验收进行中）

本轮按独立文件所有权并行，清单和最终提交统一维护。未发版、未部署，不能把本地 nginx 结果冒称生产已经生效。

### 9.1 安全与依赖

提交：`7332f03b`（web/SVG/XML/crypto），`c7cd8ad7`（文件/剪贴板 provider），`6af7dc59`（FRB 2.13 与 Rust 安全依赖图），`18cdada3`（响应头、CSP、双锁 audit 接线）。

两份 nginx 均添加安全头且覆盖 location/错误响应，`server_tokens off`；应用 CSP **只 Report-Only**，分享页保留上游原 CSP。真实容器合同与 39 项响应头自测通过，登录和实际公式/Mermaid worker 页面应用 CSP 违规为 0。自动化 `waitForFunction` 自身的 eval 注入不算产品违规，未因此开放 unsafe-eval。生产域名的响应头仍待下一次明确授权发版部署后检查。

ctutils 0.4.4，rusqlite 0.41.1 / libsqlite3-sys 0.39.1；两个锁均更新。SQLx 0.9 的可选 SQLite 约束阻碍解析，根 workspace 仅使用官方已合并修复提交 `10462fa4414c6c7f1ed1d39bb31678e3b726878c` 的 sqlx-sqlite 补丁；未启用 SQLx SQLite 运行后端，不另造数据库门面。能修复的其余公告和 yanked 版本也定向更新。无修复/未编译 transport 的例外按公告 ID 逐项说明于 `.cargo/audit.toml`，不是整 crate 忽略。adler、pin-utils 随上游依赖更新移除，不保留无用例外。cargo-audit 0.23.0，两个独立依赖图 `--deny warnings` 零未排除告警；真实 ctutils 旧漏洞锁仍退出 1。store 221 例、重建后的 FFI 指纹与本地操作 30 例通过。

XML 7.1 与 Flutter pin 所锁 meta 冲突，选可解析的 7.0.1。新版 window/tray/screen 改用 nativeapi 后端，真实调用在当前 Dart 3.13.1 AOT 编译器触发 `Class with illegal cid, full-aot` / snapshotter -6；保留 window_manager0.5.2、tray_manager0.5.3、screen_retriever0.2.2，未来 SDK 整体升级后再验证，不以 legacy ignore 掩盖不兼容。其他依赖批次的 Windows 原生 PNG/编辑器粘贴、五组 clipboard round-trip 已通过，窗口/托盘/真实文件对话框矩阵继续补验。

### 9.2 体积、加载与 wasm 评估

生产 AI、版本历史、关系图采用 `deferred as`，数据常量独立避免重新拉入首屏；下载失败留在当前页面，可重新打开重试。旧账号/工作区/文档的迟到下载不打开新页面的对话框，也不向新归属显示旧错误。对应 3 项回归与通知账号归属测试通过。JS 分片使用 no-cache，缺失分片 404。

构建样本：同期依赖下 eager 样本 `main.dart.js` **5,034,888 B**，deferred 样本 **5,012,438 B**；三个分片 15,090 / 6,529 / 11,076 B。合并其他功能后的最终首屏数值会由最终构建核对，不把所有依赖变化都归因延迟导入。专用浏览器验收 **12/12**：首屏无 part；三个真实窗口首开各加载分片、热开不重复请求；冷上下文拦截分片显示失败提示，放行重开恢复。只读状态采集与前台实际控件同时核对，未用模拟函数冒充 UI。见 [验收记录](assets/redundancy-third-2026-11-11/deferred-results.json)。

**wasm 结论：本轮不切换。** 固定 Flutter3.47.1 实际 `--wasm` 构建成功，Chrome 实验：

| 构建 | COOP/COEP | 实际执行资产 | 页面就绪单次观测 | 页面目标传输 |
| --- | --- | --- | ---: | ---: |
| dart2js | 无 | main.dart.js + CanvasKit | 633 ms | 4,727,266 B |
| dart2wasm | 无 | main.dart.wasm + skwasm | 349 ms | 4,367,232 B |
| dart2wasm | same-origin / require-corp | main.dart.wasm + skwasm | 337 ms | 4,369,782 B |

这是同机独立冷浏览器上下文、禁 HTTP cache 的单次实验，OS/V8 编译缓存未统一清空，非统计性能承诺。无隔离时没有专用 worker；隔离时实际出现 skwasm worker。隔离 harness 中原有数学和 Mermaid 两个真实 Rust wasm worker 也渲染成功、无页面异常。harness 的页面 CDP 字节数不包含所有 worker 传输，不能冒充含图页面总量。证据：[完整实验](assets/redundancy-third-2026-11-11/wasm-results.json)、[worker 对照](assets/redundancy-third-2026-11-11/wasm-workers.json)。

COOP/COEP 可影响跨域资源与窗口关系，需覆盖独立资源源站、认证和嵌入后才有上线结论；当前同源资源验证不等于任意部署都能隔离。官方[Wasm 文档](https://docs.flutter.dev/platform-integration/web/wasm)包含更新 SDK 的检测条件，不直接套用到本项目 pin；Safari 必须实测；本轮真实 Firefox159 在有/无隔离两种场景均能打开页面，但实际请求 main.dart.js + CanvasKit，并未选择 wasm/skwasm，不能把能打开当成该路径已获 Firefox 验证。Android 冷启动无设备，未据猜测调整首帧字体时序。

### 9.3 外壳、编辑器与渲染接口设计

`a44ed9ce`：主库 **14,606 → 11,992 行**，侧栏/页行/面包屑和外壳面板成为独立 library；启动/旧配置迁移、展示错误映射、编辑器投影独立；不是改成更多 part。编辑器 **7,059 → 6,737 行**，流式行内 AI 对话框独立，数据面与编辑合并规则仍由 Rust 持有。预算相应收紧。目标测试 22 例、模块归属/错误重试 7 例、通知路由绑定服务端并使用更新凭据 2 例通过；全套门禁继续运行。

本轮没有新增 AtomicBlockRenderer，不提前改渲染机制。设计草案：当前 `block_renderers.dart` 直接访问 host 的 18 个私有名字（含方法，不能沿用旧“26 个字段”当新实测），主要是选择/编辑/hover、图片与预览、平移缩放和布局。

1. 抽独立 `BlockLayoutContext`（节点、约束、选择、外观）和 `BlockPaintContext`（viewport、只读交互快照、资源借用），不能把 RenderDocument 装进 Context 再公开全部字段。
2. 资源缓存及 ui.Image 生命周期仍由现有 pipeline 持有。绘制只记录需求，异步结果继续验证 generation/ticket，渲染器不得自行 dispose 借用图片或跨 await 修改 host。
3. 原子布局结果是渲染器自有的几何对象，向 host 暴露通用 bounds、可见/吸收范围及 hit-test；表格单元格内部几何留在表格模块，不能为了拆库公开 `_TableCell`。
4. 交互输出既有 TableHit/ImageHit 等意图，由编辑器/控制器处理；不在渲染器里新增 Markdown 或 CRDT 写入规则。正文文本 painter 的复用、视口裁剪、光标/引用背景仍由 host 管。
5. 下次新增块时逐个迁移 divider→image/math→table，保留注册表及“layout 返回 null 时回退源码”的契约；每步校验尺寸/hit-test、主题/选择/表格编辑、图片释放时序与长文布局复用，再收紧实际库预算。

这是接口设计而非已实现拆库，不宣称当前 render 库已降低到软上限。

### 9.4 工程、通知及发布材料

`492d8abb`：后台导入持久化终态后给原账号发送完成邮件，独立 users 偏好列避免旧客户端外观 blob 覆写而重新开启。设置→数据可关闭；回调绑定打开时的账号/服务器，token 刷新仍使用新凭据。真实 ZIP HTTP 上传→Rust 后台任务→Postgres 终态→DirectMail 签名 HTTP 请求→本地捕获邮件，2项回执；UI 4项通过。网络发送在数据库事务外；单次 best-effort at-most-once，失败记 failed，崩溃可能留 sending，无自动重试；不把日志 Mailer 当真实投递。

`e6c0b0dd`：macOS CLI 变成默认关闭的可选输入；发布后从真实附件下载、校验平台 manifest 与实际版本，Windows 在一次性官方 runner 安装并验证首帧和10秒存活，Linux CLI/可选deb解包验证。installer 独立 `sha256-windows-installer.txt`，两个生产者都生成；CLI updater 的 sha256-windows.txt 不变。部署后 site canary 从目标 tag 取固定入口合同，新 tag 强制执行；旧 tag 仍保留旧 verify 恢复通路，明确新增 canary 未执行，不把成功恢复误报失败。

真实本地主应用/deb版本0.14.4、首帧1001ms、存活10秒；新deb体积24,037,492 B，再次从临时目录解包验证通过。这次下载索引是本地资产 fixture，不是新发布的发行版，不冒称新发布 canary 已运行。正式发布仍须用户明确要求。

工程、签名、SDK发布、yrs和LFS材料补在本节后续；Android实签、真实新发布附件和部署后响应头验收保持待用户/下次发版，不会为了打勾实际发布。

### 9.5 补充只读评估（不直接修改产品）

数据库：生产未启用慢语句日志、pg_stat_statements或IO计时，API最近10000行仅含数字的慢搜索过滤为0条，不能据此证明无慢查询。生产只做元数据和不执行语句的generic EXPLAIN，不读正文/用户字段、不 ANALYZE。页树与增量现有索引覆盖主要过滤/游标。

独立PG18.6复制真实表/索引，合成40万views（20工作区×2万）、20万updates；全量单工作区页树29.326ms/18,948行、LIMIT50为7.468ms；单工作区候选检索7.622ms，20工作区检索123.874ms；回收站4.886ms；100条文档增量0.320ms。缓存/HTTP序列化不是生产状态，不把合成值当生产延迟。只在回滚事务内试建精确匹配排序 partial index，LIMIT50变0.061ms、消除Sort；当前客户端默认全树，不能将该比例当默认页树收益。建议先建立观测，再跟踪跨大工作区标题ILIKE扫描与精确排序；未直接改查询、生产索引或VACUUM。证据：[摘要](assets/redundancy-third-2026-11-11/database-synthetic-summary.json)、[可复验合成脚本](assets/redundancy-third-2026-11-11/database-synthetic-bench.sql)。

无障碍：Chromium真实AX中桌面/390px登录有命名按钮与文本框，8次Tab无表单陷阱；但207块自绘正文无相应标题/列表/代码结构，2×2表格无table/row/cell语义，是明确缺口，不冒称真读屏正常。浅色/深色faint普通字号文本对比度 **2.529:1 / 4.233:1**，低于4.5；边框是否必须3:1与24px触摸目标间距例外要逐控件判断，不以单个尺寸直接断言全站违规。对比度[数据](assets/redundancy-third-2026-11-11/contrast.json)。

中英ARB各769业务key，集合一致；ICU格式分支不是参数缺失。扫描无直接中文Text/tooltip/hint硬编码；找到AI slash项 `Ask AI` 在中文环境可能落入英语默认标签的实际静态链，未直接修。见[文案数据](assets/redundancy-third-2026-11-11/localization.json)。

手机390×844触摸登录实际调用API后进入应用，侧栏打开页面、表格触摸进入TEXTAREA并输入通过；390px桌面引擎硬件键盘的正文输入和键盘选区通过。设置的外观/返回/关闭触摸可达；没有真实手机虚拟键盘、IME、长按把手和系统菜单验证，不把尺寸仿真当真机。正文焦点自动化一度未稳定同步，后用明确输入动作复验，未把测试工具不稳定直接报为产品故障。

### 9.6 GPUI 剩余关卡

Firefox159.0.1/geckodriver0.37.1实际打开当日NativeShell RichEditor，在编辑器加载前报 `error loading dynamically imported module: nsh:rt`，input/textarea/contenteditable为0；同URL当日Chromium152正常。最小诊断证明Firefox先注册import map再加载模块成功，先执行module再注册map失败；官网runtime采用后者，属于这份当日示例的Firefox兼容失败，非仅驱动或全站宕机。未执行的输入/composition/撤销复制/输入框位置不能记通过。证据：[摘要](assets/redundancy-third-2026-11-11/firefox-summary.json)、[最小诊断](assets/redundancy-third-2026-11-11/firefox-importmap.json)、[错误截图](assets/redundancy-third-2026-11-11/firefox-nsh-load-error.png)。

gpui-kit同版Web示例正在从官方v0.9.1固定提交39ab849重建，独立nightly工具链，不改Mica rustc1.98 pin；不因缺旧产物直接宣称不能测。Windows真人IME、Android真机、Safari仍没有本轮通过证据，不用JS合成事件、Android模拟尺寸或WebKit代替。四关维持未完整通过、不迁UI的决策；本地存储两份实现继续暂缓合并。


### 9.7 需用户操作与外部条件的材料

#### Android 稳定签名（8.5；准备完毕，密钥操作仍需用户）

来源：JDK 自带 keytool 本地生成自签名 APK 签名证书，不需要向 CA 付费。GitHub Actions 构建仍消耗其既有额度，与签名工具是否收费不是一回事。见 [Android 官方签名说明](https://developer.android.com/studio/publish/app-signing)。

首先尝试找回旧构建者的 key（原固定构建机 `.android/debug.keystore` 或原 `mica-client-android-key` 卷），用户本人只读导出到仓库之外的私密目录。**先验证旧 APK 的签名指纹与该 key 的证书 SHA-256 一致，且 keystore 中确有 PrivateKeyEntry（不仅是公钥证书），再复用**。指纹命令：

```sh
apksigner verify --print-certs OLD.apk
keytool -list -v -keystore EXISTING.keystore -alias EXISTING_ALIAS
```

只有找不到旧 key，用户才新建稳定 key，例如（命令仅准备，agent 没执行，也没生成测试签名 key）：

```sh
keytool -genkeypair -keystore /PRIVATE/DIR/mica-release.jks -storetype JKS \
  -alias mica-release -keyalg RSA -keysize 3072 -validity 36500
```

交互输入身份与密码；不要把密码放命令参数、聊天或仓库。保存离线备份。用户亲自把 key base64 和三项凭据分别配置成 GitHub Actions secrets：`MICA_ANDROID_KEYSTORE_BASE64`、`MICA_ANDROID_KEY_ALIAS`、`MICA_ANDROID_STORE_PASSWORD`、`MICA_ANDROID_KEY_PASSWORD`；可选 repository variable `MICA_ANDROID_STORE_TYPE` 为 JKS/PKCS12，默认 JKS。

实现：Gradle release 只用稳定传入 key；不完整配置、不可读文件、非法 store type 拒绝；task graph 防止 aggregate assemble 绕过 release 检查，debug 保留原 debug key。Docker 只读挂载 key，密码用 env 名传递，不放命令参数、源码或缓存卷。官方 runner 在 RUNNER_TEMP 写临时 key，并始终清理。

**若换新 key，旧安装不能直接覆盖。** 在没有旧私钥建立签名延续的情况下，必须先导出/同步并核对未上传的本地数据，再迁移或卸载旧版；不能承诺透明升级，卸载会清理私有数据。用户尚未创建/提供/上传真实 key，本机无 Java/Android SDK，**未声称新 Gradle 实签和 APK 覆盖升级已验证**。Bash 配置预检的正常/未设置/空/空白/文件不存在/类型非法 fixture 均通过，无密钥生成。

Context7 文档工具返回连接错误，已回退官方 [Gradle TaskExecutionGraph](https://docs.gradle.org/current/javadoc/org/gradle/api/execution/TaskExecutionGraph.html) 与 [AGP 9.1 SigningConfig](https://developer.android.com/reference/tools/gradle-api/9.1/com/android/build/api/dsl/SigningConfig)。

#### yrs 上游核对（8.5）

截至本次核对，[#415](https://github.com/y-crdt/y-crdt/issues/415) 仍 OPEN，最后更新 2026-08-04；[官方 v0.28.1](https://github.com/y-crdt/y-crdt/releases/tag/v0.28.1) 已于 2026-11-04 发布，其比较差异含 V2 解码边界等修复，不能仅凭 issue 未关闭就忽略补丁。

实测：在 `/tmp/mica-yrs-0281-probe` 独立 crate 强制 yrs **0.28.1**，复用当前 mica-core，原样复制四条性质测试，原配置每条 256 cases；生产 Cargo/锁/ignore 未改。敌意输入测试进程仅加 2 GiB/60 秒安全限额，非正常构建限流；**0.26 秒内 0 通过、4 失败**，仍有 `block.rs:93` 断言 panic，并非限额触发。发生反例即收缩退出，不宣称跑满 4×256 全部样本。日志 `/tmp/mica-yrs-0281-fuzz.log`，临时锁确认为 yrs0.28.1。

结论：**还不能启用“永不 panic”四条 property**，生产保留现有 containment 和两类已修 abort 回归。不将局部解码修复说成整个解码器无 panic；继续等待上游并复验。

#### 字体 Git LFS（8.5）

结论：本轮不改 LFS，也不重写历史。当前 `web/gfonts` **21,804,333 B**，原“约10MB”已经过时；`git rev-list --objects HEAD -- ... | git cat-file --batch-check` 核对有 **726 个历史 blob，原始总量21,804,333 B**，目前并无多代字体膨胀。WOFF2 已压缩，普通 build checkout 仍要获取当前所有字体，换 LFS 不会自动减少这些必需载荷。

代价：多一个 LFS 下载前提和带宽/存储配额；GitHub 官方当前 Free/Pro 含 10 GiB，超额预算为0时会阻断传输，不应为构建引入新的付费/阻断路径。正常 CI/源码包/Docker 必须正确 smudge，否则把文本指针当字体打包；现有 mirror 仅测文件非空，单靠它不能识别 LFS 指针。引入前还须增加实际 WOFF2 magic/hash 验证、调整 checkout LFS 和源码包约定。历史迁移还涉及公开提交重写，未获授权。

依据：[GitHub LFS 计量与额度](https://docs.github.com/en/billing/concepts/product-billing/git-lfs)。未来若多次引擎升级造成历史增长、实际 clone 数据证明瓶颈，再单独评估，不能把运行时按需字体下载与 Git 仓库存储混为一谈。

#### mica-md 发布材料（8.6；准备，不发布）

候选源码版本仍为 crate/npm **0.1.0**，由用户确认；`publish=false` 未改，CHANGELOG 未擅自填发布日期，未 bump、tag、cargo/npm publish。crates.io/npm 的匿名公开 `mica-md` 查询本次均 HTTP404，只表示当时未找到，不是已预留包名或已取得发布权。

现有 `scripts/sdk-check.sh` 真跑 **ALL GREEN**：Cargo 包白名单35文件、all-features rustdoc -D warnings、公开API/schema/never_panic、无默认特性全套、4个示例、双WASM/npm测试及 npm pack --dry-run（13文件）。日志 `/tmp/mica-sdk-preflight-round3.log`。候选变更说明：初次公开Rust/JS SDK；CommonMark/GFM与现有方言、块模型/schema、UTF-16 marks；Rust唯一解析/序列化/HTML实现；严格HTML profile及明确的保真profile边界，不宣称保真HTML自动移除所有脚本。

由用户按 S5 手册确认版本、审阅 CHANGELOG、从干净发布提交翻 publish 开关、打 `mica-md-v<SDK版本>` tag；用户在自己的 crates.io/npm 登录环境分别发布。App版本 `vX.Y.Z` 与SDK tag不同，凭据不进仓库/CI。正式发布前重跑 sdk-check，本次 dirty 工作树包清单检查不是正式发布的干净树证明。

#### issue #170（8.6，外部条件未满足）

[Issue #170](https://github.com/weironz/mica/issues/170) 仍 OPEN，最近更新2026-11-08。当前GitHub作者与两条历史评论均为 `weironz`，没有新增可复现操作步骤。历史17类操作Chromium复测未复现，Safari隐藏输入框线索已有#171修复说明；不能代替 Safari/报告人的实际确认。本次没有发评论、重复催促或关闭issue，保留等待补充，不能标成已复现修复。


### 9.8 最终集成门禁（持续更新）

本地完整 `just ci` 已通过：Root工作区及真实PG、mica-core store、Flutter分析/单元widget、浏览器专用套件与五个独立浏览器回归（含deferred）均实际执行，2109个前端单元/widget通过。安全审计扫描两个锁并拒绝warnings；无新增跳过或预算放宽。

独立提交：`a44ed9ce`外壳/编辑器与延迟加载，`e6c0b0dd`工程与产物验证，`0f8a7bff`延迟窗口真实浏览器验收，`d398a621`Windows原生透明PNG/编辑器粘贴，`b1789140`显式选择dry-run可选包。下一步为官方完整CI与含Linux deb的dry-run；不在本轮发版或部署。
