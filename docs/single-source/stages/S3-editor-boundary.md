# S3 编辑器边界改走引擎，删除 Dart 块级解析与序列化（5–7 天）

## 目标

编辑器所有「Markdown 进来 / 出去」的地方都改调 `markdownCodec`：

| 方向 | 场景 | 现在（Dart） | 改为 |
| --- | --- | --- | --- |
| 进 | 粘贴 Markdown / HTML（经 `htmlToMarkdown`）、AI 插入、AI「当前页」 | `markdownToBlocks` | `markdownCodec.parseBlocks` |
| 进 | 单行粘贴里的行内公式 | `parseInlineMath` | `markdownCodec.parseInlineMath` |
| 出 | 复制的 Markdown（`selectionText`）、AI 输入、`documentMarkdown` | `_rangeMarkdown` | 切片 → `markdownCodec.serializeBlocks` |
| 出 | 复制的 text/html（`selectionHtml`、单元格选区、表格复制） | `selectionHtml` / `inlineToHtml` | 切片 → `markdownCodec.renderClipboardHtml` |
| 辅助 | `htmlToMarkdown` 里的数学区间 | `mathRunSpans`（Dart） | `markdownCodec.mathRunSpans` |

然后删除不再有调用者的 Dart 镜像（03 §2 标为 **S3-删** 的全部内容）。行内解析器 `parseInline` 和 `inlineToMarkdown` 在本阶段**仍被表格路径使用**，S4 删。

## 前置

- S2 全部验收通过（门面在两个平台可用、契约测试通过）。
- 在**切换前**的提交上生成 S0 T0.5 的剪贴板快照与差分报告（作为对比基准）。

## 任务顺序与停点 G3

任务编号不等于执行顺序。按下面的顺序做，**中间在 G3 停下**（[04 §4.2](../04-parallel-plan.md)）：

```
T3.1 → T3.2 → T3.3 → T3.4(R) → T3.5 → T3.7 → T3.8 → ⏸ G3 → T3.6 → T3.9 → T3.10 → T3.11
```

G3 时的状态：编辑器**已经切到引擎**，但 Dart 旧实现**还没删**——所以 T3.7 的分诊能拿新旧两套结果直接对比，
审核方和用户能在单向门之前看到全部行为变化。G3 的停点材料（写进本文执行记录）：

1. T3.4 剪贴板移植验收（`clipboard_html_port_test`）的回执；
2. T3.7 分诊表全文，以及从中提炼的「用户可见变化」「待用户决定」两节；
3. T3.8 往返测试的回执与例外清单；
4. `bash scripts/local-ci.sh all` 回执。

G3 之后才删代码（T3.6）。S4 的 T4.1–T4.3 可以在 G3 之前做，T4.4 起要等 T3.6 合并。

## 任务

### T3.1 准备：搬走保留的 Dart 粘贴启发式

- 新建 `lib/editor/paste_repair.dart`，把 `retagMislabeledFences`、`unwrapNestedFences` 从 `markdown.dart` **原样剪切**过来
  （连同它们只给自己用的私有辅助函数）。文件头注释：

  ```dart
  /// Paste-only repairs for Markdown that LLM chat UIs emit (a second equal-length
  /// fence around a code block; Python labelled ```bash). They run on the pasted
  /// TEXT before it reaches the engine and are deliberately not part of the
  /// grammar: file import, MCP writes and the server parse strictly. Kept in Dart
  /// because `retagMislabeledFences` leans on `highlight.dart`'s language
  /// detection. docs/single-source/02-architecture.md §2.1, decision D7.
  ```

- `editor.dart`、`controller.dart`、`highlight.dart` 改 import。`ai_edits.dart`、`editor.dart` 的 `BlockSpec` 改从
  `../markdown/markdown_codec.dart` 导入。
- 提交：`refactor(editor): T3.1 move paste repairs out of the markdown mirror`。全部测试照常通过（纯搬家）。

### T3.2 解析入口切换（7 处）

| 文件 | 位置（约） | 改法 |
| --- | --- | --- |
| `editor.dart` | 1268（单行富文本内联合并） | `final specs = markdownCodec.parseBlocks(markdown);` |
| `editor.dart` | 1297（多行 / 富文本粘贴） | `_controller.insertBlocksReplacingSelection(markdownCodec.parseBlocks(markdown));` |
| `editor.dart` | 1787、5664、5756 | 同上 |
| `editor_ai.dart` | 383 | 同上 |
| `main_shell.dart` | 2987（`_aiCurrentFromMarkdown`） | 同上（`main.dart` 库行数净变化 ≤ 0） |

- 模式：一律 `ParseMode.Document`（`fragment: false`）——粘贴一个带 front matter 的 `.md` 时丢掉元数据，与现在 Dart 的行为一致
  （`test/front_matter_test.dart`「Metadata is dropped on paste」）。
- 降级：粘贴路径（`_handleRichPaste`）用 `try { … } on MarkdownCodecException { return false; }` 包住，返回 `false` 让调用方
  落到现有的纯文本分支（02 §5.2）。AI 路径捕获后显示现有的错误提示。**不要**在每个调用点各写一份 catch：
  在 `editor.dart` 加一个私有辅助 `List<BlockSpec>? _parseOrNull(String md)`，失败时记一次日志并返回 `null`。
- `\r\n` 归一化（`editor.dart` 约 1144 行）保持在调用引擎之前。

### T3.3 行内公式与数学区间

- `editor.dart` 约 1232：`final parsed = markdownCodec.parseInlineMath(plain);`（返回 `InlineText`，字段名与原 record 相同）。
- `html_to_markdown.dart` 约 56：`final spans = markdownCodec.mathRunSpans(s);`，删掉 `import 'marks.dart' show mathRunSpans;`。
- 验证：S1 T1.5 已把 `paste_formula_test.dart` 里直接测 `parseInlineMath` 的用例移植成 Rust 测试；本任务把 Dart 侧这些用例改为调门面
  （B 类测试），全部通过。

### T3.4 Rust：实现剪贴板 HTML 方言（`HtmlProfile::Clipboard`）

在 `crates/md-core/src/html/clipboard.rs` 实现，`render_html_body` 在 `profile == Clipboard` 时调用它。
**规格 = 现在的 Dart 实现**：逐行阅读并 1:1 移植

- `clients/mica_flutter/lib/editor/controller.dart` 的 `selectionHtml`（约 1633–1798）与 `_tableHtml`（约 1799–1822）；
- `clients/mica_flutter/lib/editor/marks.dart` 的 `inlineToHtml`、`_renderSpanHtml`、`escapeHtml`、`escapeHtmlAttr`（约 2563–2682）。

已知必须保留的约定（不完整，以 Dart 源码为准）：

| 块 / 标记 | 输出 |
| --- | --- |
| `math_block` | `<p><span data-mica-math="1">$$…$$</span></p>`（LaTeX 原文 HTML 转义） |
| 行内 `math` 标记 | 照 `_renderSpanHtml` 的 `data-mica-math` 写法 |
| `footnote_def` | `<p data-mica-fndef="label">…行内…</p>` |
| 脚注引用 | `data-mica-footnote` 属性 |
| 代码块 | `<pre><code class="language-x">…</code></pre>`；语言取 `data.language`（Dart 预处理已填好，见 02 §2.1） |
| mermaid | **保持代码块**（`language-mermaid`），绝不输出 SVG |
| 列表 | 连续、同引用深度、完整选中的列表项按 `data.indent` 输出**嵌套** `<ul>/<ol>`；`todo` 的复选框写法照 Dart |
| 引用 | 按 `data.quote` 深度开合嵌套 `<blockquote>`（Dart 的 `setQuote`） |
| 表格 | `<table>`，单元格内容走行内 HTML |
| 图片 | `<img src alt>`，src 取 `image_srcs[file_id]`，否则 `data.url` |

部分选中的块（例如只选了段落中间几个字）：切片在 Dart 侧完成（T3.5），Rust 只看到切好的块，所以 Rust 侧**不需要**选区逻辑；
但 Dart `selectionHtml` 里「`full` 选中才保留引用深度」之类的判断依赖选区，移植时改为：Dart 构造 BlockSpec 时，对不完整选中的块
**去掉** `quote` / `qbreak`（与 Dart 现有 `setQuote(full ? depth : 0)` 等价），并在注释里写明对应关系。

测试：
1. `crates/md-core/tests/clipboard_html.rs`：每条约定至少一例，期望字符串取自当前 Dart 输出。
2. **移植验收（Dart，切换前跑）**：`test/clipboard_html_port_test.dart`——对 30 个 fixture 文档全选，断言
   `markdownCodec.renderClipboardHtml(切片块)` 与 `controller.selectionHtml()`（旧 Dart 实现）**逐字节相同**。
   这个测试只在 T3.5 删除 Dart 实现之前存在；不同之处要么修 Rust，要么在执行记录里逐条说明为什么接受差异，然后才能进入 T3.5。

### T3.5 复制 / 导出改走引擎

在 `controller.dart` 新增：

```dart
/// The selection as engine blocks: each slice of [_selectionSlices] becomes a
/// [BlockSpec] carrying only the selected text, with marks clipped to it.
/// Partially selected blocks lose their quote grouping (the old HTML writer's
/// `setQuote(full ? depth : 0)`). Code blocks get the language the copy should
/// carry ([_copyLanguage] — auto-detected, Dart-only, 02 §2.1).
List<BlockSpec> selectionBlocks() {
  final out = <BlockSpec>[];
  for (final sl in _selectionSlices()) {
    final node = nodes[sl.node];
    final full = sl.from == 0 && sl.to == node.text.length;
    final data = Map<String, dynamic>.from(node.data);
    if (!full) {
      data.remove('quote');
      data.remove('qbreak');
    }
    if (node.kind == 'code_block') {
      final lang = _copyLanguage(node);
      lang.isEmpty ? data.remove('language') : data['language'] = lang;
    }
    final marks = clipMarks(marksFromData(node.data), sl.from, sl.to);
    marks.isEmpty ? data.remove('marks') : data['marks'] = marksToJson(marks);
    out.add((kind: node.kind, text: node.text.substring(sl.from, sl.to), data: data));
  }
  return out;
}
```

- `clipMarks(marks, from, to)`：放进 `marks.dart` 的**模型部分**（留下的那部分）。逻辑从 `CellEditController.selectionHtml`
  （`cell_edit_controller.dart` 约 46–70 的裁剪循环）提取，两处共用。原子块（表格、图片、分隔线）的切片 `from/to` 为整块，`clipMarks` 原样返回。
- `selectionText({imageUrls})`：保留「光标在原子块上则选中整块」的现有逻辑，然后
  `return markdownCodec.serializeBlocks(selectionBlocks(), assets: imageUrls ?? const {});`
- `documentMarkdown({imageUrls})`：对全部节点构造 BlockSpec（不切片，语言同样预处理）→ `serializeBlocks`。
- `selectionHtml({imageUrls})`：`return markdownCodec.renderClipboardHtml(selectionBlocks(), imageSrcs: imageUrls ?? const {});`
- `CellEditController.selectionHtml()`：`renderClipboardHtml([(kind: 'paragraph', text: 选中文本, data: {'marks': …})])`。
- `editor.dart` 约 4105 的表格复制 HTML：`renderClipboardHtml([(kind: 'table', text: '', data: node.data)])`。
- 降级：`selectionHtml` / `selectionText` 捕获 `MarkdownCodecException` 返回 `''`（`_copySelection` 已经处理「HTML 为空只写纯文本」）。
- `selectionClipboardText`（text/plain）**不变**，其中表格单元格的 `parseInline(cell).text` 暂时保留（S4 改）。

### T3.6 删除

以下全部删除（先 `grep -rn` 确认零调用者，结果贴执行记录）：

- `lib/editor/markdown.dart`（整个文件）
- `lib/editor/list_serialization.dart`（整个文件）
- `table.dart`：`looksLikeGfmTable`、`_startsBlock`、`_splitRow`、`tableToMarkdown`
- `controller.dart`：`_rangeMarkdown`、`_blockPrefix`、`_continuationPrefix`、`_codeFenceFor`、`_tableHtml`，以及 `selectionHtml` 的旧函数体
- `marks.dart`：`escapeBlockLeader`、`escapeBlockLeaders`、`escapeMarkdownTitle`、`escapeMarkdownDestination`、`escapeInline`、
  `escapeHtml`、`escapeHtmlAttr`、`inlineToHtml`、`_renderSpanHtml`、`parseInlineMath`、`mathRunSpans`（若仍有使用者，留到 S4 并记录）
- `markdown.dart` 删除后，`isThematicBreak` 等随文件消失

删除后：`flutter analyze` 零 error/warning；`bash scripts/check-file-size.sh` 通过（各库只减）。

差分工具（S0 T0.2）同步收缩：块解析、Markdown 序列化、剪贴板 HTML 三类比较的 Dart 一侧已不存在 → 从
`markdown_differential_test.dart` 删掉这三类，`differential_baseline.json` 只保留行内类别（`parseInline` / `inlineToMarkdown`，S4 删除）。
删除前最后一次的完整报告贴进执行记录。

### T3.7 行为差异分诊

材料：S2 版差分报告（`build/differential/`）+ 切换前后的剪贴板快照对比（T0.5 的命令在切换后再跑一次，diff 两份 JSON）。

每一条差异归入一类，写进执行记录的分诊表（id、类别、输入摘要、旧输出、新输出、结论）：

| 结论 | 含义 | 处理 |
| --- | --- | --- |
| R：Rust 正确 | 旧 Dart 行为是 bug 或偏离规范 | 接受；若影响用户可见行为（粘贴结果变了），列入「用户可见变化」小节 |
| H：Dart 启发式 | 旧行为是刻意的粘贴/复制体验（D7） | 在调用引擎前后用 Dart 预/后处理保留（像 `paste_repair.dart`）；写明理由 |
| B：Rust bug | 新行为错 | 在 `md-core` 修，配回归测试（先证明会失败），提交 `fix(md-core): …`；修完重跑差分 |
| U：待用户决定 | 产品取舍拿不准 | 先按 Rust 行为，列入「待用户决定」表 |

**完成标准**：块级、行内、序列化、剪贴板四类差异全部有结论；B 类全部修复；差分棘轮基线更新为分诊后的结果。

### T3.8 Mica 内复制粘贴往返测试（剪贴板方言的守护）

`test/clipboard_mica_roundtrip_test.dart`（B 类，走门面）：对每个 conformance fixture：

```dart
final blocks = markdownCodec.parseBlocks(md);
final html = markdownCodec.renderClipboardHtml(blocks);
final back = markdownCodec.parseBlocks(htmlToMarkdown(html));
expect(normalize(back), normalize(blocks));   // 规范化：marks 排序、data 中仅比较 schema 键
```

已知不能往返的（例如 HTML 不表达的 `loose`），在测试里列为显式例外并写注释；例外清单同时贴执行记录。
这个测试是 Rust 写端 ⇄ Dart 读端（`htmlToMarkdown`）这条跨语言契约的唯一守护，**必须进门禁**。

### T3.9 测试迁移

按 03 §3 标为 S3 的文件处理：A 类迁到 Rust 后删除（执行记录写对应的 Rust 测试名）；B 类加 `setUpAll(useNativeMarkdownCodec)` 后保留；
C 类（`markdown_conformance_test`、`markdown_export_conformance_test`）删除——它们被 S2 的门面契约测试取代。

### T3.10 真实路径验证（AGENTS.md：桌面问题桌面验证，Web 用 Playwright 截图）

| 平台 | 场景 | 证据 |
| --- | --- | --- |
| Windows 桌面 | 从浏览器复制富文本粘贴；从 ChatGPT 复制带双围栏代码块粘贴；粘贴带 front matter 的 `.md` 内容；Mica 内复制含公式/脚注/嵌套列表/表格的段落再粘贴；复制到 Typora / Word | 截图 + 前后对比说明 |
| Web（Chrome，生产构建） | 同上的 Mica 内往返；从外部页面粘贴 | Playwright 截图（可复用 `e2e/` 的 harness 页面） |
| 两端 | AI「读取当前页」→ 发送的 Markdown 与改前一致（或差异已在 T3.7 分诊） | 日志片段 |

### T3.11 门禁

`bash scripts/local-ci.sh all` 全绿；浏览器套件（含契约测试）通过；Windows 集成测试 `markdown_codec_test.dart` 通过；回执贴执行记录。

## 验收

- [ ] `grep -rn "markdownToBlocks\|_rangeMarkdown\|listSerialization\|markdownListLayout\|tableToMarkdown" clients/mica_flutter/lib` 无结果。
- [ ] `lib/editor/markdown.dart`、`lib/editor/list_serialization.dart` 已删除。
- [ ] `clipboard_html_port_test` 曾在切换前通过（执行记录有回执），之后随旧实现一起删除。
- [ ] `clipboard_mica_roundtrip_test` 通过并在门禁中。
- [ ] T3.7 分诊表完整，B 类全部修复，「用户可见变化」与「待用户决定」两节已写。
- [ ] 桌面与 Web 的人工验证证据齐全。
- [ ] `main.dart` 及其他登记库的行数只减不增。

## 风险

| 风险 | 应对 |
| --- | --- |
| 剪贴板方言移植不全，Mica 内复制粘贴丢格式 | T3.4 的逐字节移植验收 + T3.8 往返测试 |
| 粘贴结果的细微变化（列表松紧、空行）被用户感知 | T3.7 分诊，「用户可见变化」写进发版说明 |
| 某个 Dart 预处理被遗漏（像代码语言识别） | 移植验收逐字节比较会暴露；新增的预处理统一放 `paste_repair.dart` 或 `selectionBlocks()`，不散落 |
| 门面不可用时粘贴整体失效 | T3.2 的降级 + 测试：`debugSetMarkdownCodec(const UnavailableMarkdownCodec())` 下粘贴退回纯文本 |

## 执行记录

（执行方追加：分诊表、用户可见变化、待用户决定、删除行数统计、证据。）

### 当前结论（2026-10-28，G3 停点）

| 项 | 状态 |
| --- | --- |
| T3.1–T3.5 | ✅ 编辑器全部 Markdown 进出走 `markdownCodec`；`lib/` 中 `markdownToBlocks` 调用点 7→0 |
| T3.4 剪贴板移植验收 | ✅ `clipboard_html_port_test` 30 fixture 逐字节（切换前最后验证于 `9657631d` 链上） |
| T3.7 分诊 | ✅ 全部差异归类：B 类 1（已修）、R 类 6、H 类 3、U 类 1；见下表 |
| T3.8 往返测试 | ✅ 35 个 fixture 中 29 个逐项比较（其中 12 个只抹掉已知损失项）、6 个确实无法比较（审核方 G3 改写，见文末）；原为 18 个整文件跳过 |
| 全量测试 | ✅ `flutter test` 2232 passed / 21 skipped / 0 failed（Windows）；审核方 Linux 复跑 `98c71d14`：2210 passed / 29 skipped / 0 failed，浏览器套件 9 组通过 |
| `local-ci.sh all` | ✖ **原回执不成立**：`dd183384` 上 `cargo fmt --check` 失败（`crates/md-core/tests/wire.rs`，自 `7ef6bbba` 起），`flutter analyze` 有 5 个 warning（3 个旧函数无引用、2 个多余 import）。审核方已修格式与 import；3 个旧函数随 T3.6 删除 |
| 增删行数（`origin/main..98c71d14`） | 产品代码 `lib/` +397 / −369；测试 +314 / −15；差分基线 JSON +300 / −7,240；Rust +118 / −125。原写「−7,460」几乎全是基线重置，旧 Dart 实现尚未删除（T3.6） |

### 提交与门禁回执

| 提交 | 任务 | 内容 |
| --- | --- | --- |
| `e7b4a533` | T3.1 | `paste_repair.dart` 粘贴围栏启发式搬家；editor_ai/ai_edits 改从门面取 `BlockSpec` |
| `71b27bc2` | T3.2+T3.3 | 7 处解析入口 + `parseInlineMath`/`mathRunSpans` 切门面；`_parseOrNull` 统一降级 |
| `7ef6bbba` | B 类修复 | wire 不再对 marks 重排序（分诊表 B-1） |
| `3b6c477f` | T3.4/T3.5/T3.8 | 复制/导出切引擎；`selectionBlocks`/`documentBlocks`/`_visibleOrdinal`；`clipboard_mica_roundtrip_test` |
| `dd183384` | T3.4 收尾 | `table_copy.dart` 拆分；预算基线收紧（editor -15、controller 4139→4063） |

### T3.7 分诊表

材料：切换前后 `clipboard_snapshot_test` 双快照（`build/s3-baseline/`，不提交）、差分报告前后对比。**export 1432→19、roundtrip 1165→1190 的收敛是 wire marks 顺序修复（B-1）的直接结果**；差分棘轮已按修复后基线重置。

| # | 类 | 差异 | 结论 |
| --- | --- | --- | --- |
| B-1 | B（Rust bug） | wire `parse_body_json` 按 `(start,end,type)` 重排 marks；序列化 tie-break 依赖解析器产出序（同 range 外层 mark 排后面），深嵌套（fixture 43）wire 往返 ≠ native 导出 | **已修**（`7ef6bbba`）：wire 保留解析器产出序、只排 JSON 键；回归测试 `wire_round_trip_matches_native_export_on_equal_range_ties`（先证失败后修）；wire gold 两绑定同步再生 |
| R-1 | R | 引用组序列化：Rust 写 `> line\n>\n> next\n>`（每段重新前缀 + 组尾裸 `>`），旧 Dart 写软换行折叠 | 接受。往返更保真：粘回仍是每源行一块；copy_markdown_test 已按新行为断言 |
| R-2 | R | 列表子段空行：Rust 在 loose item 的第二段前保留 `- ` 空项行，旧 Dart 只写空行 | 接受。粘回块结构一致 |
| R-3 | R | 图片块序列化：Rust 的 alt 取 `block.text` 但在原子块只复制整块时旧路径丢了 alt（`![](url)`）；现在 selectionBlocks 原子块整块复制，alt 保留 | 接受（是修复） |
| R-4 | R | 编号列表中间起复制：旧 Dart 需 `markdownListLayout` 全文扫描才能给出可见序号；新路径 `_visibleOrdinal` 回写 `data.start` | 接受，行为一致 |
| R-5 | R | 表格区域复制的 `<td>` 内容：旧路径对 cell 文本 `escapeHtml`；新路径经引擎 Clipboard 方言（同一语义） | 接受；windows_clipboard_copy_test 通过 |
| R-6 | R | HTML 方言软/硬换行、链接/图片 title、`*` 标记 flavor、callout 类型、>32 层嵌套不往返 | 接受：全部是旧 Dart 写出器同样丢失的（见 T3.8 例外清单，逐项注明）；非 S3 回归 |
| H-1 | H（Dart 启发式保留） | `retagMislabeledFences`/`unwrapNestedFences`（LLM 粘贴修复） | `paste_repair.dart`，引擎调用前（D7） |
| H-2 | H | `_copyLanguage` 代码语言检测 | `selectionBlocks()` 在构造 BlockSpec 时写入 `data.language`（02 §2.1） |
| H-3 | H | `\r\n` 归一、裸图片 URL 检测、单元格内粘贴原文 | 保留在原调用点，引擎之前 |
| U-1 | U（已决定 2026-10-28） | 部分选中代码块的复制：旧 Dart 返回裸片段（无围栏），新路径保留 code_block kind，引擎输出围栏包裹 | **用户选「带围栏」**（选项 1，即 Rust 现行为）：粘贴回保真、仍是代码块；粘进代码块内部的场景不受影响（该路径本来就走原文直插，不经过解析）。已定案 |

### 用户可见变化（进发版说明）

1. **复制引用/多段列表再粘贴，块结构更保真**：旧实现把引用组软换行折叠成一块、把 loose 列表第二段的 `- ` 空项行丢掉；新实现逐块保真（R-1/R-2）。
2. **复制图片块再粘贴，alt 文字保留**（R-3，实为修复旧丢失）。
3. **部分选中代码块复制会带围栏**：旧实现给裸文本，粘贴回可能散架；新实现围栏包裹（U-1 已决定：用户选带围栏，2026-10-28）。
4. **HTML 剪贴板方言的固有限制不变**（title 丢失、软换行扁平化等，R-6）——与 S3 之前一致，无回归。

### 待用户决定

| # | 问题 | 决定 |
| --- | --- | --- |
| U-1 | 部分选中一个代码块后复制，剪贴板里是否带 ``` 围栏？ | **带围栏**（用户 2026-10-28 拍板，选项 1）：粘贴回仍是代码块；粘进代码块内部的场景不受影响（`_handleRichPaste` 对块内选区走原文直插，不经过解析） |

（无待决项。）

### 差分棘轮

`differential_baseline.json` 重置（`S0_WRITE_BASELINE=1`）：B-1 修复使 export 1432→19、roundtrip 1165→1190（roundtrip 上升是因为 export 修复后不再遮蔽 roundtrip 维度的既有差异，净差异 1209 < 原 2597）。前后报告存 `build/s3-baseline/`。

### G3 停点声明

编辑器已切到引擎，Dart 旧实现（`markdown.dart` 1491 行、`list_serialization.dart`、`controller._rangeMarkdown` 主体、`selectionHtml` 旧函数体）**未删**，T3.6 在 G3 通过后执行。⏸ 停在这里等审核方与用户。

### G3 必须修改：审核方完成（2026-10-28）

按用户要求，G3 的三条必须修改由审核方直接做完，执行方不用再做。提交见 `single-source` 上的「G3 fixes」提交。

| # | 改动 | 回归测试（均先证明撤掉修复会失败） |
| --- | --- | --- |
| 1 | `EditorController.documentMarkdown()` 引擎不可用时不再返回 `''`，改为抛出 `MarkdownCodecException`；`_openAiDialog` 经门面新增的 `withEngineOrNull` 读取正文，拿不到时不提供「当前页面」目标（也关掉 AI 编辑预览）。`main.dart` 库净 0 行 | `ai_current_page_context_test`：`documentMarkdown never stands in an empty body…`、`without the engine the AI dialog offers no current-page target` |
| 2 | `clipboard_mica_roundtrip_test` 改为按损失项定点抹平：`title`、`softBreak`（换行折空格并重算标记偏移）、`marker`、`loose`、`listStart`、`alert`、`rawHtml`、`quoteGroups`、`emptyQuote`、`quoteItalic`。整文件例外从 18 个降到 6 个（18、20、33、38、40、43，各写明为何无法对齐） | 本测试 29 个 fixture 逐项比较全绿 |
| 2′ | 改写后立刻抓到读回端 `htmlToMarkdown` 的真 bug：字面 `!` 紧挨链接（`\![x][ref]` → `!<a>x</a>`）读回成 `![x](u)`，链接变图片。修复：文本节点末尾的 `!` 写成 `\!` | `paste_inline_marks_test`：`a literal ! right before a link stays text, not an image`；往返 `16-images.md` |
| 3 | 本表的增删行数与门禁回执改正 | — |

改写时新列出的四类既有损失（S3 之前就有，读回端或写出端行为未变）：相邻有序列表合并成一个 `<ol>` 导致起始编号丢失（`listStart`）、相邻引用组合并（`quoteGroups`）、单独一行 `>` 的空引用块丢失（`emptyQuote`）、引用内斜体被读回端**刻意**去掉（`quoteItalic`，`c1185887` 的粘贴启发式，针对 Word / 公众号把整段引用排成斜体的样式）。

另：`cargo fmt` 修正 `crates/md-core/tests/wire.rs`；删除 `highlight_languages_test` / `paste_formula_test` 的多余 import；`file-size-budget.txt` 中 controller 基线 4063 → 4062。

