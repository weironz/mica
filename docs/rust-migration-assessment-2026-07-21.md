# Rust 化评估：哪些 Dart 模块该、能、值得换成 Rust

**日期**：2026-07-21 ｜ **方法**：四路只读子代理并行扫全仓 + 主会话核对
**触发**：CLAUDE.md 原则 #2「Rust-first 数据面」的一次系统性体检

## 证据等级

- 【实测】本会话用命令/探针跑出来的
- 【子代理】只读代理读源码得出，主会话抽查未反证
- 【未证实】有理由相信，但没有测量支撑 —— **不作为决策依据**

---

## 先纠正一个流传的错误前提

CLAUDE.md 转述的理由是「web 跑不了我们的 Rust FFI」，本报告起草时我也照此推理。**这是错的。**

`docs/architecture.md:25-36` 写的是真实理由：

> Editor hot paths stay client-side by design. Live input rules (`**b**` as you type),
> paste-to-blocks, copy-as-markdown (clipboard APIs must produce data inside the user
> gesture) cannot take a network round trip per keystroke.

而且同一段已经写明长期路径：**"compiling the Rust engine to WASM for the client"**。

**铁证**【子代理】：`lib/editor/marks.dart` 被 `render.dart` / `controller.dart` / `editor.dart` / `cell_edit_controller.dart` 导入，**没有一个是平台条件导入**。Windows 桌面端**已有完整原生 FFI**，却仍然每帧都在用 Dart 镜像。

→ **「web 不行」不是原因。真正的约束是热路径延迟，它对桌面同样成立。上 WASM 不会自动删掉这些代码。**

这条记在这里，是因为它差点让整份评估的结论跑偏 —— 正是 CLAUDE.md #6 说的那类「必须 X 才能 Y」前提。

---

## 家底【实测】

Rust **28,294** 行 / Dart **46,474** 行（除去 `l10n` 与 `src/rust` 生成码）。

| Rust crate | LOC | | Dart 目录 | LOC |
|---|---|---|---|---|
| api-server | 8,611 | | **editor** | **21,048** |
| markdown | 6,472¹ | | lib 根 | 8,454 |
| mica-core | 4,424 | | ui | 4,333 |
| app-core | 3,202 | | cloud | 2,383 |
| interchange | 2,318 | | api | 1,989 |
| mcp-server | 1,812 | | local | 1,479 |
| cli / infra | 1,455 | | web / upload / widgets | 955 |

¹ = `src/lib.rs` 5,349 + `tests/*.rs` 1,123。**引擎本体是单文件 5,349 行**。

`lib/editor` 21,048 行的分桶【子代理】：

| 桶 | LOC | 说明 |
|---|---|---|
| **B — UI/渲染/输入** | 11,757 | 自绘 RenderBox、IME、手势、平台粘合 —— 必须留在 Dart |
| **A — 纯数据变换** | 3,048 | Rust 候选 |
| **C — 混合** | 6,243 | controller 3,095 / marks 1,950 / highlight 1,095 |

把 C 里的纯逻辑拆出后，**A 形状代码约 6,340 行，占该目录 30%**。

---

## 分类结论

### 甲类：已有 Rust 孪生，**不在热路径** —— 现有模式即可去重

| Dart | LOC | Rust 对应 | 备注 |
|---|---|---|---|
| `upload/zip_writer.dart` | 102 | `interchange/src/zip/writer.rs`（114） | **两份手写 ZIP 二进制编码器**，须逐字节兼容 |
| `main.dart` 本地世界 CRUD（L2280–3260） | ~980 | `documents.rs` 递归 CTE 处理器 | 注释自认 *"Mirror the server's restore_view"* |
| `local_offline_io.dart` `cloneView`/`_dedupName` | ~150 | `clone_view` / `dedup_sibling_name` | 注释自认 mirrors the server's |
| `main.dart` 三处 markdown 正则 | ~50 | `strip_notion_id` / `strip_leading_h1` | 违反原则 #2/#4，量小但漂移风险高 |

**存在性证明已在仓库里**：`clients/mica_flutter/rust/src/api/store.rs:368 export_folder_zip` —— FFI 暴露的 Rust 函数内部调用**和服务端完全相同**的 `mica_interchange::build_markdown_tree_zip`，Dart 只递本地 CAS 的图片字节。文件夹导出早已这样去重。**甲类是照抄这个已验证模式，不需要任何新技术。**

`zip_writer` 应最先动：手写二进制格式重复两份，一旦漂移，产出是**打不开的 zip**。

### 乙类：已有 Rust 孪生，**在热路径** —— 需要 WASM + 实测延迟

Markdown 引擎：Dart **5,447** 行 ↔ Rust `lib.rs` **5,349** 行，**~52 对镜像函数**【子代理】。

可删约 **2,400–2,800 行**（Dart Markdown 面的 45–50%）：

- `markdown.dart` ~1,128 / 1,410（`markdownToBlocks` 单函数就是 `import_markdown` 的 853 行镜像）
- `marks.dart` ~755–816 / 1,950
- `table.dart` ~86–108 / 270

WASM 事实【子代理，含官方文档核实】：

- flutter_rust_bridge **2.12.0 官方支持 web**（`build-web` 子命令），且 `lib/src/rust/frb_generated.web.dart` **712 行早已生成在仓库里**，只是没人消费
- **体积不是障碍**：现有 web 产物 59 MB，其中 `mermaid.min.js` 单个 **2.57 MB**；~1 MB 的 wasm 占比 < 2%
- **代价在别处**：强制 nightly Rust + `-Z build-std`（每次重编 std）、COOP/COEP 响应头（等于强制 HTTPS）、`catch_unwind` 在 web 失效（本会话刚用它兜 yrs panic）、`std::thread::spawn` 不可用
- **一个具体阻塞点**：`crates/markdown/src/lib.rs:1895` 的 `Uuid::new_v4()` 在 `wasm32-unknown-unknown` 上需 `getrandom` 的 js backend，**现状编不过**

### 丙类：**无** Rust 对应物 —— 要写新代码

| Dart | LOC | 说明 |
|---|---|---|
| `highlight.dart` | 1,095 | `crates/` 里 `syntect`/`tree-sitter` 零命中 |
| `html_to_markdown.dart` | 722 | 无任何 HTML 解析库（`html5ever`/`scraper` 零命中） |
| `mermaid_svg_inline.dart` | 443 | Rust `render_mermaid_svg` 输出带 `<style>` 的 SVG，拍平是 Dart 独有 |

≈ **2,260 行**。这才是字面意义的「用 Rust 替代」，但**不解决任何现存 bug**，纯属数据面归位。

### 丁类：web 侧结构性重复 ~1,600 行

`mica_ydoc.dart` 437 + `web_idb_doc_store.dart` 540 + `cloud_sync_web/io` 两份（~80% 结构重叠）。卡在 `docs/phase2-offline-crdt.md:133`「core crate 刻意不编进 web」这个决定上。不重新审视该决定就动不了。

---

## 最高 ROI 的不是上面任何一类

**conformance 同步机制有结构性盲区。**【子代理，主会话核对】

Dart 侧 `markdown_conformance_test.dart` 读 Rust 生成的 gold JSON、跑 `markdownToBlocks` 比对 —— **只覆盖 import 方向**。Dart 侧从未被跨引擎校验的有：

- 导出方向（`inlineToMarkdown` / `escapeBlockLeader` / `escapeInline`）
- HTML 导出
- `htmlToMarkdown`
- 语法高亮

而 Rust 的 `fixtures_round_trip` 测的是 **Rust 自己的**导出器。**两端导出路径之间没有任何交叉校验。**

这直接解释了两次漏网：

1. `escapeBlockLeader`（P1-1 唯一当场实测确认的漂移）是**导出侧**漂移
2. 2026-07-21 修的序列化器丢链接（`` [`code`](url) `` → 丢 URL），是 Rust round-trip **偶然**抓到的，不是机制抓到的

**关键含义**：fixture 从 23 → 31 对，把 import 方向覆盖率从 3.6% 提到 4.8%，**但导出方向仍是 0%**。继续加 fixture 改变不了这一点 —— 这是机制形状问题，不是语料量问题。

---

## 执行计划（按 ROI 排序）

### 第 1 步：补导出方向 gold ✅ 已完成（本次）

Rust 侧 `GEN_GOLD` 为同一批 fixture 额外产出 `.md.gold` = `export_markdown(import_markdown(md))`（新增 `fixtures_export_match_gold`，31 份 gold）；Dart 侧新增 `markdown_export_conformance_test.dart`，用 `EditorController.load()` + 全选 + `selectionText()` 比对同一份 gold。**未改任何生产代码**。

#### 机制一上线就抓到东西【实测】

32 个 fixture 里 **15 个导出结果不一致**（另有 1 个是 harness 假阳性，已修：`~~~a~~` 解析成单个空文本 code_block，选区必然折叠，`selectionText` 按契约返回空串 —— 已改为 skip）。

按 `commonmark_scoreboard.rs` 的做法设**回归地板**：15 个列入 `_knownDivergent` 白名单待分诊，**其余 17 个立即生效守着**，CI 保持绿，白名单只能缩小。

差异分两簇，**尚未分诊，不要假定每条都是 bug**：

- **列表/引用簇**（03、04、05、15、17、20）—— 差在空行与松散列表的位置。**可能是合法差异**：Rust 的 gold 是 `export_markdown`（整文档导出），Dart 的是 `selectionText`（复制到剪贴板），本就是两个产品。
- **行内簇**（11、13、16、19、21、22、23、34）—— 更可疑。`34-link-title-escape` 直接丢掉 link title（`[a](/u "C:\name")` → `[a](/u)`），**和 2026-07-21 修的 `render_span` 丢链接是同一种数据丢失形状**。已确认 Dart 序列化器代码里**有** `pick.title` 分支，所以丢失点在别处，未继续追。

> 这 15 条的分诊 = 把本次会话追那 12 处漂移的过程再走一遍，是独立一轮的工作量。**但机制已经就位** —— 在此之前，这 15 处差异一个都不会有人发现。

### 第 2 步：甲类去重

`zip_writer` 优先（模式已验证、无新技术、漂移后果最重），再 `cloneView`/`_dedupName`、markdown 正则，最后本地世界 CRUD（~980 行，量最大、和 `setState` 交织最深）。

### 第 3 步：WASM spike ✅ 已完成（2026-07-21）—— **延迟这条否决理由不成立**

阈值**在测量之前**定死（免得事后找理由）：< 1 ms 可行 / 1–5 ms 灰色 / > 5 ms 否决。

实测【实测】，同一批输入喂两边：

| | Dart | WASM（含 JSON 序列化） | WASM（仅解析） |
|---|---|---|---|
| **热路径** 87 字符 | 0.059 ms | **0.029 ms** | 0.019 ms |
| **批量** 10,935 字符 | 3.642 ms | **1.801 ms** | 1.574 ms |

**WASM 比 Dart 快约一倍**，且已含跨边界与 JSON 开销。热路径 0.029 ms，低于阈值 34 倍。

体积：**wasm 201 KB + JS 胶水 4.5 KB**（`opt-level="z"` + LTO）。现有 web 产物 59 MB、其中 `mermaid.min.js` 单个 2.57 MB —— 这个 wasm 是**那一个资源的 1/13**。此前报告里「~1 MB」是从 comrak-wasm 外推的【未证实】推断，**偏悲观，此处修正**。

`Uuid::new_v4()` 阻塞点属实但很小：`uuid` 需要它**自己的** `js` feature（只开 `getrandom/js` 不够）。一个 target-gated 的 feature 即可。

#### spike **没有**证明的事（同样重要）

- **测的是 Node/V8 + 纯 wasm-bindgen，不是 Flutter web。** dart2js/dart2wasm 调 wasm 另有一层 interop 开销，未测。
- **201 KB 不是 frb 的数字。** frb 的 `build-web` 用 `-Z build-std` + 线程标志，产物结构不同、更大。
- **工程代价未量化**：强制 nightly Rust、COOP/COEP 响应头（等于强制 HTTPS）、`catch_unwind` 在 web 上失效（本会话刚用它兜住 yrs panic）。
- **没有解释桌面端**：Windows 早就有原生 FFI 却仍用 Dart 镜像。那条路的开销是另一回事。

#### 结论

`docs/architecture.md` 给出的承重理由是「热路径不能每次按键过一趟往返」。**就 wasm 而言，这条经测量不成立** —— 它比现状还快。

但「可行」不等于「现在就做」。下一步不是删那 2,400 行，而是第二个 spike。

### 第 3b 步：frb web spike ✅ 已完成（2026-07-21）—— **可行，但 web 上没有性能收益**

第一个 spike 测的是 Node + 裸 wasm-bindgen。这一个把 frb 的 web 路径在**真实 Chrome** 里跑通，并且——这点关键——**对照组换成同一个浏览器里的 Dart**（先前拿 Dart VM 比浏览器，是拿 JIT 比浏览器，证明不了任何事）。

两轮样本，同一台机器、同一个 Chrome、同一批输入【实测】：

| | Dart→JS（浏览器） | frb wasm（浏览器，含 JSON） | frb wasm（仅解析） |
|---|---|---|---|
| **热路径** 87 字符 | 0.085 / 0.046 ms | 0.093 / 0.079 ms | 0.083 / 0.046 ms |
| **批量** 7,118 字符 | 4.22 / 6.38 ms | 5.73 / 5.43 ms | 4.74 / 5.62 ms |

**结论：在浏览器里两者基本打平**，算上 JSON 序列化 wasm 还略慢一点。第一个 spike 里「快一倍」那个优势，在 frb 的编解码层 + dart2js 本身够快之下消失了。

产物体积（frb 真实 `build-web`，含 `wasm-opt`）：

- wasm **180 KB 原始 / 78 KB gzip**
- JS 胶水 **30 KB 原始 / 6 KB gzip**
- 合计传输量 **~84 KB**

比第一个 spike 的裸 wasm-bindgen 版**还小**（201 KB → 180 KB）。此前说 frb 产物「结构不同、更大」的推断【未证实】**也是错的，此处修正**。

#### 一个结构性阻碍（实测）

现有 FFI crate `rust_lib_mica_flutter` **整体无法上 web**：

```
Compiling libsqlite3-sys v0.35.0
error: failed to run custom build command for `libsqlite3-sys`
```

它依赖 `mica-core` 的 `store` feature = `rusqlite` + `bundled`（要编 C 版 SQLite）。而更本质的是**浏览器里没有本地 SQLite 文件存储**——整个 `MicaStore` 面在 web 上没有意义（`local_offline_web.dart` 那 207 行空壳正是这个原因）。

→ 要把 Markdown 引擎搬上 web，**必须另建一个只暴露 markdown 的 web 专用 bridge crate**，而不是复用现有的。这不是可选项，是前提。

#### 工程代价（已实测，不再是推断）

- nightly 工具链 + `rust-src` 组件（`-Z build-std` 要它）
- `wasm-pack`
- COOP/COEP 响应头是**硬要求**：默认静态服务器不设，得自己加（本次要手写一个 server 才跑得起来）〔第 5 步复审修正：这只是**默认线程化构建**的要求，frb 官方有免线程路径，见下〕
- 构建耗时：cargo ~55s + wasm-pack ~67s，且 `-Z build-std` 每次重编标准库

#### 结论

三条否决理由：**延迟不成立**（打平）、**体积不成立**（84 KB）、**「web 跑不了」不成立**（跑通了）。

但**支持它的理由也只剩一条**：少维护 2,400 行重复。代价是 nightly 工具链 + 第二个 bridge crate + COOP/COEP + 一条新的 web 构建链路。

**且这条理由今天变弱了**：双表示危险的根源是静默漂移，而 import 与 export 两个方向现在都有共享 fixture 守着（本日建立，坏用例已清零）。漂移会被自动抓住，不再是「迟早出事」。

**建议：可行但暂不推进。** 真要做，触发条件应当是「又出现了 fixture 抓不到的漂移」或「要新增一个平台」，而不是「因为能做」。

### 第 4 步：丙类 —— 最后，或永远不做

收益纯属"数据面归位"，不修任何现存 bug。真要动，`html_to_markdown` 优先于 `highlight`（前者在粘贴路径上、后者只影响显示）。

### 第 5 步：丁类复审 ✅ 已完成（2026-07-21）—— 拆成两半：丁-1 纯 Dart 今天就做，丁-2 继续挂起

4 路并行调查（前提提取 / 重复解剖 / AppFlowy 扒真实源码 / COOP-COEP 代价核查），结论**改写了丁的性质**：它不是一个「要不要换引擎」的决定，是两个独立问题。

#### 前提核对：`phase2-offline-crdt.md:133` 的理由已全部过期

- 决定原文的括号理由「web 仍走现有云端 API 路径」——写下 **2 天后**即失效（d147184 建 `cloud_sync_web.dart`，web 切到 yjs CRDT 路径）；「web 不需要离线」死于 P4-2（cdd1caf）；「wasm 慢 / 大 / 跑不了」死于 §3/§3b 两个 spike。git blame：决定文本自 a112cf5（2026-06-06）起一字未改。
- 红线 #4「数据面权威单点留在 Rust，别被『AFFiNE 用 Yjs 也行』诱回 JS」——P4-2 之后 web 恰恰长成了 AFFiNE 形状（JS yjs 数据面 + 1,600 行 Dart 驱动 + IndexedDB）。当初豁免的支撑（wasm 不可行）已被证伪，只剩工程代价一条腿。

#### 解剖【实测】：「统一引擎才能合并 sync 逻辑」被证伪

两份 sync session 机械 diff：io 456 / web 446 非注释行，其中 **407 行逐字节相同**（LCS）；引擎只透过 **8 个方法**被触碰（每文件真正碰引擎的只有 ~25-30 行）。三类划分：

| 类 | 行数 | 内容 | 消除条件 |
|---|---|---|---|
| 只有换引擎才消得掉 | ~680 | `mica_ydoc.dart` 437（**mica-core doc/marks 语义的 Dart 镜像**，M4.7 字段级 props 被迫实现两遍的就是它）+ yjs 胶水 ~145 + 兼容验证负担 ~100 | 丁-2 |
| 平台固有，任何方案消不掉 | ~690 | IndexedDB vs SQLite——且这两份**本就不是逐行重复**，是同一契约（`cloud_doc_store.dart`，已抽好）的异构实现 | 无 |
| **不动引擎今天就能抽** | **~630** | **整个同步状态机**（bootstrap 合并规则、ack 连续前缀、poison-edit 熔断、重连退避、outbox、compaction、persist 防抖——红线 #1 的语义现在存在两份，靠人肉同步，web 版注释里写了 15 次 "mirrors io"）+ op 重放 ~50 | 丁-1 |

#### 参照系（AppFlowy / AFFiNE，扒真实仓库源码）

- **AppFlowy 生产架构 = 桌面 yrs + web yjs + 服务端 yrs——与我们现状完全同构**（AppFlowy-Web package.json: `yjs@14.0.0-1` + `y-indexeddb`，零 wasm；AppFlowy-Collab: `yrs 0.25`）。它**实弹试过**全 Rust wasm（`frontend/appflowy_web` af-wasm 全家，跑了约 6 个月），2024-07 整体删库（#5671），2024-11 连只剩网络层的 wasm client 也删（AppFlowy-Cloud #989）。
- **ywasm**（yrs 官方 wasm 绑定）：免 COOP/COEP（单线程 wasm-bindgen），363 KB gzip（完整 API 面），维护活跃但**用户全是基准套件，零生产应用**。
- **AFFiNE y-octo**：止步服务端 napi、`experimental.yocto` 默认关、d.ts 里明写 round-trip 兼容问题未解决前不得回发 yjs 客户端——**双引擎架构的风险集中在字节兼容面**，一次实证不够，要回归地板压着。
- 排除法结论：现状不是权宜之计，是被生产验证过的架构（AppFlowy-Web yjs ↔ AppFlowy-Cloud yrs 天天互通）。窄核 wasm（我们 84 KB 的那种）**无先例可抄但也未被证伪**——AppFlowy 失败的是全后端 wasm 化，不同类。

#### COOP/COEP 修正：不是硬成本

- frb 官方文档有「Run without cross-origin headers」路径：`default_dart_async: false`（全 `#[frb(sync)]`，只跑主线程）+ `--wasm-pack-rustflags` 去掉默认线程化标志 → **完全不需要 COOP/COEP**。对我们反而天然契合——要上 web 的 markdown 引擎本就是同步调用形态。运行时 frb 对非隔离页面只 warn 不炸。（免 COOP/COEP ≠ 免 nightly：`build-web` 硬编码 `-Z build-std`；绕开 build-web 用 stable wasm-pack 可免，spike #1 已实证。）
- 即便真要上，对本项目破坏面逐项核查后很小：无 OAuth 弹窗（纯 JWT 表单）、字体全打包、图片走 CORS 模式 XHR 自绘（RustFS CORS 已在用）、CanvasKit gstatic 实测带 CORP 头。真正的工作量在 nginx 头作用域纪律：`add_header` 继承陷阱 + `/s/` 分享页必须豁免（第三方热链 `<img>` 会被站级 COEP 拦掉）；「只对 wasm 路径加头」原理上不可行（`crossOriginIsolated` 由顶层文档响应头决定）。
- 顺手发现（与丁无关但该修）：构建没加 `--no-web-resources-cdn`，CanvasKit 实际从 gstatic CDN 拉取，打包在 `deploy/web/canvaskit/` 的副本是死重——建议补上该 flag，消除对 Google 持续提供 CORP 头的依赖 + 国内访问 gstatic 的不确定性。**已修（2026-07-21）**：justfile `dev-web`/`build-web`、ci.yml、release.yml 四处 `flutter build web` 统一加 flag；实测新 buildConfig 带 `useLocalCanvasKit:true`，:8090 冒烟 CanvasKit 走同源 `canvaskit/chromium/`、页面渲染正常、console 零错误。残留：引擎**字体回退**（Noto Sans SC 等）仍在运行时从 `fonts.gstatic.com` 拉取——与 CanvasKit 无关的另一条 gstatic 依赖，断网只影响缺字形回退不致命，要消除得走字体打包/FontManifest，另行评估。

#### 判决

- **丁-1【做】**：抽 8 方法 `SyncDocReplica` 接口 + 平台工厂（照 `doc_store_platform` 的条件导入模式），两份 session 合成一份共享（~750 行）+ 桌面适配器 ~50 行 + web 适配器 ~20 行（`MicaYDoc` 当初就是按镜像 `MicaDocument` 写的，方法一一对应）。红线 #1 的语义从两份人肉同步变一份。零新工具链、零部署改动;做完后若将来上 wasm，只换一个适配器，session 与 store 全不动——**两步解耦，互不阻塞**。
  〔✅ 已落地（当日）：`cloud_sync_io.dart`(739) + `cloud_sync_web.dart`(611) = 1,350 行 → `cloud_sync_session.dart`(747) + 契约 44 + 两适配器 57/48 + 门面 6 = 902 行，净 -448；外部消费者(main.dart 等 9 处)零改动。验证：analyzer 干净、676 单测、7 个 sync 集成测试 + 4 个 FFI 集成测试全绿。**验证过程的意外收获**：基线实验(旧代码同样失败)把 `cloud_sync_test` 的"随机"失败追到一个真服务端 bug——并发首次 bootstrap 的 ensure_base 竞态让输家客户端拿到平行 CRDT 宇宙的 base，对端编辑永远 pending 且 applyUpdate 返回 Ok（红线 #1 静默分歧），修复 + 确定性回归测试见 commit `eaefb6e`；顺带补齐 FFI workspace 漏升的 yrs 0.27.3（`1a68142`）。〕
- **丁-2【挂起，带触发条件】**：换引擎消那 ~680 行。它的漂移面是真实的（`doc.rs`/`marks.rs` 每次演进都要人肉同步 `mica_ydoc.dart`，且**没有**乙类那样的 fixture 守护），但参照系全部反向、无先例可抄。触发条件：`mica_ydoc` 镜像再出漂移 bug、yjs/yrs 升级破坏字节兼容、或参照系出现窄核先例。
- **独立必做（不论哪条路）**：yjs↔yrs 字节兼容是依赖豁免 #7 的承重前提，但唯一的跨引擎测试 `web_interop.rs` 是 `#[ignore]` 状态（要手工从浏览器捕获 `MICA_WEB_STATE_B64`），**无任何 CI 守护**。任一端升级改编码都不会被自动抓住——这正是 AFFiNE y-octo 踩过的面。要固化成回归地板。

---

## 本报告的局限

- 甲类 #2（本地世界 CRUD ~980 行）的体量是**估算**：逻辑与 `setState` 在 `main.dart` L2280–3260 交织，未逐行分离，可提取的纯逻辑核心**小于** 980 行。
- ~52 对镜像函数中约 10 对是"语义等价、命名不同"，**未逐一 diff 函数体**，对数 ±3。
- 乙类那个 wasm 体积估计（~1 MB）是**类比 comrak-wasm 的推断【未证实】**；frb 的 `-Z build-std` + 线程运行时会推高体积，**没有任何公开的 frb-wasm 体积数据**。只有本地实际编一次才能定。
- ~~丁类三项均未重新审视 `phase2-offline-crdt.md` 的决定本身。~~（2026-07-21 已复审，见第 5 步。）
