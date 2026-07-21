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

### 第 3 步：WASM spike —— **只量延迟，不动生产代码**

把 `crates/markdown`（关掉 `render` feature）编成 wasm，测按键路径实际耗时。**数字出来之前，删那 2,400 行不该拍板。**
先解掉 `Uuid::new_v4()` 的 wasm 兼容。

### 第 4 步：丙类 —— 最后，或永远不做

收益纯属"数据面归位"，不修任何现存 bug。真要动，`html_to_markdown` 优先于 `highlight`（前者在粘贴路径上、后者只影响显示）。

---

## 本报告的局限

- 甲类 #2（本地世界 CRUD ~980 行）的体量是**估算**：逻辑与 `setState` 在 `main.dart` L2280–3260 交织，未逐行分离，可提取的纯逻辑核心**小于** 980 行。
- ~52 对镜像函数中约 10 对是"语义等价、命名不同"，**未逐一 diff 函数体**，对数 ±3。
- 乙类那个 wasm 体积估计（~1 MB）是**类比 comrak-wasm 的推断【未证实】**；frb 的 `-Z build-std` + 线程运行时会推高体积，**没有任何公开的 frb-wasm 体积数据**。只有本地实际编一次才能定。
- 丁类三项均未重新审视 `phase2-offline-crdt.md` 的决定本身。
