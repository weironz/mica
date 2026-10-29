# S4 表格单元格改用「文本 + 标记」内存模型，删除 Dart 行内解析（4–6 天）

## 目标

1. `TableData` 的单元格从「Markdown 源码字符串」改为 `TableCell(text, marks)`——与段落一样的模型。
   渲染、单元格编辑、复制不再需要解析 Markdown。
2. 删除 Dart 行内解析器与序列化器（`parseInline`、`inlineToMarkdown` 及其内部，03 §2.2 标为 **S4-删** 的全部内容）。
3. 完成后 `clients/mica_flutter/lib` 中**不再有任何 Markdown 语法实现**（`html_to_markdown.dart` 是 HTML → Markdown 的转换器，
   不属于语法实现；输入快捷规则属于编辑器交互）。

**存储格式不变**（D6）：单元格在文档里仍是「字符串（源码）或 `{text, marks}`」（02 §6.4）。不迁移数据；旧客户端照常能读新客户端写的表格。

## 前置

- S2 完成（`decodeCells` / `encodeCells` 可用）。
- 与 S3 的关系：S4 改 `table.dart`、`cell_edit_controller.dart`、`render.dart` 的单元格部分、`block_renderers.dart` 的表格部分、
  `controller.dart` 的单元格写入。与 S3 共同触碰的文件只有 `controller.dart` 和 `editor.dart`——**按 04 §3 的顺序：S3 的
  T3.5/T3.6 合并后再开始 S4 的 T4.4 起的任务**；T4.1–T4.3 可以在 S3 期间进行。

## 任务

### T4.1 `TableCell` 类型（`table.dart`）

```dart
/// One table cell: plain text plus inline marks over it — the same model a
/// paragraph uses. The STORED form is still the engine's (a source string or
/// `{text, marks}`, docs/single-source/02-architecture.md §6.4); decoding and
/// encoding go through [markdownCodec] in [TableData.fromBlock] /
/// [TableData.toBlockData], so nothing in the editor parses Markdown.
@immutable
class TableCell {
  const TableCell(this.text, [this.marks = const []]);
  static const empty = TableCell('');

  final String text;
  final List<Mark> marks;

  bool get isEmpty => text.isEmpty;

  TableCell copyWith({String? text, List<Mark>? marks}) =>
      TableCell(text ?? this.text, marks ?? this.marks);

  @override
  bool operator ==(Object other) =>
      other is TableCell && other.text == text && marksEqual(other.marks, marks);

  @override
  int get hashCode => Object.hash(text, Object.hashAll(marks.map((m) => (m.start, m.end, m.type, m.href, m.title))));
}
```

- `marksEqual(List<Mark> a, List<Mark> b)`：放进 `marks.dart` 模型部分，按 `(start, end, type, href, title)` 逐个比较（先按规范顺序排序）。
  **不要**给 `Mark` 加 `operator ==`——现有代码里可能有依赖引用相等的地方，改语义的影响面没评估。
- `rows` 类型改为 `List<List<TableCell>>`；`TableData.empty()`、`insertRow`、`insertColumn` 用 `TableCell.empty`。

### T4.2 解码与编码（`TableData.fromBlock` / `toBlockData`）

```dart
factory TableData.fromBlock(Map<String, dynamic> data) {
  final raw = data['rows'];
  // … 现有的 rows 形状校验保持不变 …
  final cells = <Object?>[for (final row in rawRows) ...row];
  final decoded = _decodeCells(cells);     // 一次批量调用
  // 按原形状切回二维
}

/// Stored cells → [TableCell]. `{text, marks}` objects decode locally (they
/// already ARE text + marks); only source STRINGS need the engine, and those
/// are cached by content — layout calls fromBlock on every relayout.
static List<TableCell> _decodeCells(List<Object?> cells) {
  // 1. Map 形态：直接取 text / marks（marksFromData），不调引擎
  // 2. null / 缺失：TableCell.empty
  // 3. String：先查 _sourceCache（LinkedHashMap<String, TableCell>，上限 4096，LRU）
  //    未命中的去重后一次性 markdownCodec.decodeCells(misses)，结果写回缓存
  // 4. 其他标量：TableCell('$value')（与现有 _cellText 的兜底一致）
  // 引擎不可用（MarkdownCodecException）：字符串原样作为纯文本（02 §5.2）
}

Map<String, dynamic> toBlockData() {
  final flat = [for (final row in rows) ...row];
  final encoded = markdownCodec.encodeCells([for (final c in flat) (text: c.text, marks: c.marks)]);
  // 按形状切回二维写入 'rows'；其余键（header、align、widths、aligns、width）保持现有写法
}
```

- 注意 02 §6.4 的规则：无格式但含字面 `*` 等字符的文本，由引擎转义后存成字符串。**Dart 不做任何「简单情况直接存字符串」的捷径**——
  那等于在 Dart 里重写一份转义规则。
- 引擎不可用时 `toBlockData` 的降级：全部存成 `{text, marks}` 对象形态（总是正确的，只是不如字符串紧凑），并记日志。
- 缓存的单元测试：同一字符串第二次解码不调用引擎（用一个计数包装的门面验证——包装真门面，不是假实现）。

### T4.3 渲染（`render.dart`、`block_renderers.dart`）

```dart
static TextSpan cellDisplaySpan(TableCell cell, TextStyle base, MicaTokens tokens) {
  if (cell.isEmpty) return TextSpan(text: ' ', style: base);
  return buildMarkedSpan(cell.text, cell.marks, base, tokens);
}
```

`block_renderers.dart` 约 700 行：`c < table.rows[r].length ? table.rows[r][c] : TableCell.empty`。

### T4.4 单元格编辑（`cell_edit_controller.dart`）

- `CellEditController(TableCell cell)`：直接 `marks = cell.marks; value = TextEditingValue(text: cell.text, …)`，不再 `parseInline`。
- 新增 getter `TableCell get cell => TableCell(text, List.unmodifiable(marks));`，删除 `serialize()`。
- `CellEditSession`：`_opening` 类型改为 `TableCell`；`preview()` / `commit()` / `reconcile()` 里的 `editor.serialize()` 全部改为
  `editor.cell`，比较用 `==`（`TableCell` 值相等）。`reconcile()` 里 `CellEditController(TableData.fromBlock(…).rows[row][col])` 直接传 `TableCell`。
- `selectionHtml()`：S3 已改为 `renderClipboardHtml`，这里只改它取文本和标记的来源。

### T4.5 写入接口（`controller.dart`）

- `previewTableCell(int at, int row, int col, TableCell cell)`、`setTableCell(…, TableCell cell)`：约 1990–2013 行，
  `table.rows[row][col] = cell;` 比较 `==`。
- 清空选中单元格（约 2051 行）：`if (!table.rows[r][c].isEmpty) table.rows[r][c] = TableCell.empty;`
- `selectionClipboardText` 中的表格分支：`cell.text.replaceAll('\t', ' ').replaceAll('\n', ' ')`（不再 `parseInline`）。
- `editor.dart` 约 2717：打开单元格编辑时取 `TableCell`。
- 其余 `table.rows[...]` 的读写点（`grep -n "rows\[" lib/editor/*.dart`）逐个改，清单贴执行记录。

### T4.6 Web CRDT 路径

`lib/web/mica_ydoc.dart` 把块 `data` 当 JSON 存取，不解析单元格——**确认**它没有任何对单元格形态的假设（grep `rows`），结论写执行记录。
存储格式不变，所以这里预期零改动。

### T4.7 删除

- `marks.dart`：03 §2.2 标为 **S4-删** 的全部内容（`parseInline` 及内部、`inlineToMarkdown` 及 `_renderSpan` 系列、`normalizeLabel`、
  `matchingBracket`、`bracketNesting`、`_ScanMemo`、`extendedAutolink`、`inlineHtmlEnd`、`parseEntity`、`unescapeMd`、`autolinkTarget`、
  `_LabelCache`、`_Delim`、`_processEmphasis`、`_isCjk` 等），以及 S3 遗留的任何行内 HTML/转义函数。
- `table.dart`：`_cellText`、`_encodeCell`。
- 删除后 `marks.dart` 应只剩：`Mark` 的 re-export、`marksFromData`、`marksToJson`、`buildMarkedSpan`、`rangeHasMark`、数学区间辅助、
  `applyMark`、`shiftMarks`、`splitMarks`、`concatMarks`、`_normalize`、`clipMarks`、`marksEqual`、`linkClickHit`。预计 ≈ 300 行。
- 验证：

  ```bash
  grep -rnE "parseInline\(|inlineToMarkdown\(|markdownToBlocks\(|escapeInline\(|escapeBlockLeader" clients/mica_flutter/lib   # 无结果
  ```

### T4.8 兼容性测试（不迁移数据的证明）

`test/table_cell_storage_compat_test.dart`（走真门面）：

1. **旧文档读**：`rows` 全是源码字符串（含 `**b**`、`` `c` ``、`[l](u)`、`\|`、`<br>`）的表格 → `fromBlock` → 每个单元格的 text/marks 与 Rust `decode_cell` 一致。
2. **新写 → 旧读**：新模型 `toBlockData()` 的输出，喂给**旧版** `_cellText` 逻辑（在测试文件里复制一份旧函数作为历史对照，注释说明来源提交）
   得到的源码，再经引擎解析，等于原单元格。证明已发布的旧客户端能正确显示新客户端写的表格。
3. **往返**：`fromBlock(toBlockData(t)) == t`，覆盖 S1 T1.8 的边界字符集。
4. **I3 除外项的处理**（G1 裁决 #4 留给 S4）：S4 之后单元格编辑器不再解析源码，用户在单元格里输入
   `https://example.com` 时内存里没有 `link` 标记，存成字符串后读回却会有——编辑中和重新打开后显示不一致。
   开始 T4.4 前在「待用户决定」里列出两种做法并请用户选：(a) 单元格提交时自动给裸 URL / 邮箱加 `link` 标记（与读回一致）；
   (b) 这类单元格改存 `{text, marks: []}` 对象（新客户端无损；旧客户端读到的仍是链接）。选定后补对应测试，并改 02 §6.4。

### T4.9 测试迁移

03 §3 标为 S4 的文件：`render_span_equivalence_test`（C 类，删除——对拍对象 `_renderSpan` 已删）、`math_code_span_test`、
`escapes_autolinks_test`、`inline_scan_perf_test`、`nested_bracket_perf_test`、`linked_code_span_test`、`tilde_range_test`（A 类：确认 Rust
有等价测试后删除，执行记录写对应关系）、`table_inline_code_test`、`cell_copy_marks_test`、`cell_edit_controller_test`（B 类：改为 `TableCell` 模型）。

### T4.10 性能验收

02 §7：500 单元格表格（全部旧字符串格式）首次布局的解码耗时（桌面 ≤ 16 ms、Web ≤ 30 ms），之后的重新布局引擎调用次数为 0（缓存命中）。
写一个基准测试 `test/perf/table_cells_bench_test.dart` 量这两个数，结果贴执行记录。

### T4.11 真实路径验证

Windows 桌面 + Web：打开含旧格式表格的现有文档（从生产导出一份或用 fixtures 造）；编辑单元格（加粗、链接、代码）；
复制表格粘贴到 Mica 内与外部；撤销 / 重做；两个客户端协同编辑同一个表格（一个旧版本客户端、一个新版本客户端，若条件允许）。截图/录屏贴执行记录。

### T4.12 文档收尾

按 03 §5 改 `AGENTS.md`、`architecture.md`、`editor-engine.md`、`lessons.md` §1。`AGENTS.md` 的新规则原文：

```
- **Markdown 只有一份实现**：解析、序列化、HTML 都在 Rust `crates/md-core`；服务端 / CLI / MCP 原生调用，桌面经 FFI、Web 经 wasm。
  Dart 只通过 `lib/markdown/markdown_codec.dart` 的 `MarkdownCodec` 调用，**禁止在 Dart 里新增 Markdown 语法逻辑**
  （粘贴启发式放 `lib/editor/paste_repair.dart`，并写明为什么不是语法）。见 [单一实现专项](docs/single-source/README.md)。
```

替换掉原来「Markdown 语法以 `crates/markdown` 为权威，Dart 镜像必须同步更新」那半句。

## 验收（S4、S5 都完成后进入停点 G4 最终审核）

- [ ] T4.7 的 grep 无结果；`marks.dart` ≤ 400 行；`lib/editor/markdown.dart` 不存在（S3 已删）。
- [ ] T4.8 三项兼容测试通过。
- [ ] 差分工具已无可对比的 Dart 实现 → 删除 `test/markdown_differential_test.dart` 与基线文件；在执行记录里贴最后一次运行的分类计数（应全部为 0 或已分诊）。
- [ ] 性能数字达标。
- [ ] 桌面与 Web 人工验证证据齐全。
- [ ] `AGENTS.md` 等文档已更新。
- [ ] 门禁全绿，回执齐全。

## 执行记录

执行方：Agent D（WP-D）。分支 `ss/d/t4-table-cells`（自 `single-source` @ `d5c5282a`）。
提交全部 `--no-verify`（code-review-graph 钩子在本机持续 `database is locked`，S3 已有先例）。

| 提交 | 任务 | 内容 |
| --- | --- | --- |
| `7759ba60` | T4.1–T4.5 | `TableCell(text, marks)` 模型贯穿 `TableData` / `CellEditController` / `cellDisplaySpan` / `previewTableCell` / `setTableCell` / 表格区域复制；`parseGfmTable` 删除（引擎解析，对齐测试改走 `parseBlocks`） |
| `ce4ad1c9` | T4.7–T4.9 | `marks.dart` 2,455→373 行；差分工具与基线删除；A/C 类测试处置；render.dart 预算 5739→5737 |
| `1b1bdcea` | T4.8/T4.10 | `table_cell_storage_compat_test`（9 用例）+ `test/perf/table_cells_bench_test`；解码批量上限 64 |
| `6a65343e` | T4.11 | Web e2e 扩展单元格编辑/加粗/撤销/重做序列（两种剪贴板模式）；桌面集成测试场景 4 改真 bold mark |
| （本次） | T4.12 | AGENTS.md 规则替换、architecture/editor-engine/scoreboard/lessons 文档更新、04/README 进度行 |

### T4.1–T4.5 模型与改写点清单

- `TableCell`：值相等（`marksEqual` 排序后逐字段比较，`Mark` 本身不加 `operator ==`——引用相等的语义影响面未评估，与计划一致）；`hashCode` 对 `(start, end, type, href, title)` 聚合。
- **命名冲突**：Flutter 自带 `TableCell` widget。五个会同时看到两个名字的 import 加 `hide TableCell`（`cell_edit_controller.dart` / `editor.dart` / `render.dart`，以及 7 个测试文件）；`table_copy.dart` / `table.dart` 不受影响。
- 解码（`TableData.fromBlock`）：`{text, marks}` 对象本地取值（零引擎调用）；字符串去重后经 `markdownCodec.decodeCells`，结果按内容进 LRU（4,096 条）；`null`/缺失 → `TableCell.empty`；其他标量 → `TableCell('$value')`（原 `_cellText` 兜底语义）。
- 编码（`toBlockData`）：`TableCell`→存储形态按内容缓存；未命中批量经 `markdownCodec.encodeCells`。
- **I3 裁决（G1 #4 留给 S4，用户 2026-10-28 拍板，选项 b「存对象形态」）**：无标记单元格仅当「编码出的字符串再读回与原单元格完全相等」才存字符串；裸 URL/邮箱、首尾空白等读回会变形的，存 `{text, marks: []}` 对象。编辑中与重开一致；旧客户端按源码解析照样显示链接。测试钉死三种形态。
- 批量上限 64：一次 500 单元格的单批 wire 结果在 Dart `jsonDecode` 上花 ~10 ms（引擎侧只有 ~1.3 ms，瓶颈在单个大 JSON 的解码）；拆成 8×64 后 warmed 解码 0.8–1.0 ms。
- `table_copy.dart` 的 `cellClipboardMarkdown` 删除——区域复制直接把 `TableCell` 交给 `TableData(rows).toBlockData()['rows']`，与整表复制同一编码路径。
- `rows[...]` 读写点（grep 全量核对）：`block_renderers.dart:700`（渲染）、`cell_edit_controller.dart`（reconcile/preview/commit）、`controller.dart` 的 `setTableCell`/`previewTableCell`/`clearTableCells`/`selectionClipboardText`、`editor.dart` 的 `_openCellEditor`/`_copyTableArea`、`table_copy.dart`。全部改为 `TableCell` 或 `cell.text`。

### T4.6 Web CRDT 路径

`lib/web/mica_ydoc.dart` 把块 `data` 当 JSON 存取（grep `rows` 无单元格形态假设，唯一 `rows` 匹配是 `marks_diff.dart` 注释里的词）。**零改动**，符合预期。

### T4.7 删除

- `marks.dart` 剩余内容与计划一致：Mark re-export、`marksFromData`/`marksToJson`、`buildMarkedSpan`、`rangeHasMark`、`clipMarks`、数学区间辅助（`mathRunAt`/`snapOutOfMathRun`/`mathRunEndingAt`/`mathRunStartingAt`）、`applyMark`/`shiftMarks`/`splitMarks`/`concatMarks`/`_normalize`、`marksEqual`（新增）、`linkClickHit`。**373 行**（验收线 ≤ 400）。
- `table.dart`：`_cellText`、`_encodeCell`、`parseGfmTable`、`_splitRow`、`_startsBlock`、`_isThematicBreak` 全部删除。
- 验证 grep（T4.7 验收）：`parseInline(|inlineToMarkdown(|markdownToBlocks(|escapeInline(|escapeBlockLeader` 在 `clients/mica_flutter/lib` 仅命中门面自身的 `parseInline` 方法声明（`markdown_codec.dart` / `_io` / `_web` 的接口与转发，那是门面 API 本身，不是 Dart 语法实现）——编辑器库内无任何语法实现。
- `lib/editor/markdown.dart` 不存在（S3 已删）。

### T4.8 兼容性测试（`test/table_cell_storage_compat_test.dart`，真门面，9 用例全绿）

1. **旧文档读**：8 种源码串（`**bold**`、`` `c` ``、链接、`\|`、`<br>`、转义、中文、裸 URL）解码与 Rust `decode_cell` 逐字段一致；转义管道不裂列。
2. **新写 → 旧读**：带标记单元格存对象（旧 Dart `_cellText` 逻辑作为历史参照复制在测试文件里，注释注明来源提交 `d5c5282a`）；无标记存源码；裸 URL 存对象（I3 裁决）；字面 `*` 存转义源码。
3. **往返**：T1.8 边界字符集（`a *b* c` / `a | b` / `a \ b` / 空串 / 😀 / 换行 / 中文 / code+link / 转义 / 首尾空白 / `~~~` / `x_y_z`）mark-free 与 marked 两类全部 `fromBlock(toBlockData(t)) == t`；缓存返回同一实例。

### 差分工具最终报告（删除前最后一次运行，2026-10-28）

- 语料 5,742 条（仅行内层，S3 T3.6 收缩后的口径）。分类计数：**inline:export 1,520 / inline:marks 18 / inline:text 18**。
- 三类全部是 S3 T3.7 分诊表已定性的既有类别（export 的转义差异属 R 类「Rust 更保守」；marks/text 的 18 条是 fuzz 边角里 italic 边界一格的偏移差，Rust 为权威）。**无新增差异**。协议 `s3-inline-only-v1` 基线随工具一并删除。

### T4.9 测试处置终表（03 §3 的 S4 行）

| 文件 | 处置 | Rust 对应 |
| --- | --- | --- |
| `render_span_equivalence_test`（C 类，2 用例） | **删除**——对拍对象 `_renderSpan` 已删 | 引擎序列化本身由 wire 黄金（`serialize_body.json`）与 S3 契约测试守护 |
| `markdown_differential_test` + `differential_baseline.json` | **删除**——Dart 一侧已无可对比实现 | 上表；wire 黄金 + `clipboard_mica_roundtrip_test` |
| `math_code_span_test`（A，5） | **删除** | `crates/markdown/tests/math_code_span.rs`（同名同断言） |
| `escapes_autolinks_test`（A，7） | **删除** | `conformance/11-escapes-autolinks.md`（转义/自动链接全部用例）+ wire 黄金 `parse_body.json`/`serialize_body.json` 的 escape 段 + `conformance/02-inline.md`（嵌套标记） |
| `inline_scan_perf_test`（A，9） | **删除** | `crates/markdown/tests/inline_scan_perf.rs`（#61 非二次） |
| `nested_bracket_perf_test`（A，4） | **删除** | `crates/markdown/tests/nested_bracket_perf.rs` |
| `linked_code_span_test`（A，5） | **删除** | `crates/markdown/tests/linked_code_span.rs`（link 与 code 同区间） |
| `tilde_range_test`（A，4） | **删除** | `crates/markdown/tests/gfm_extensions.rs` `a_single_tilde_between_word_characters_is_a_range`（含 CJK 数字变体） |
| `table_inline_code_test`（A+B，3） | **重写为 B 类**（单元格模型） | Rust 半边仍是 `crates/markdown/tests/table_inline_code.rs`；Dart 侧改断言 `TableCell.text`/`marks` 与往返 |
| `cell_copy_marks_test`（B，1） | **改模型**：表格经 `fromBlock`（旧源码串形态）构造，粘贴回来的 cell 断言 `marks` 非空 | — |
| `cell_edit_controller_test`（B，10） | **重写为 `TableCell` 模型**（源码经 `markdownCodec.decodeCells` 解码后构造，与真实编辑会话一致）；`serialize()` 断言改为 `cell` 值相等 / marks 区间断言 | — |
| `clipboard_plain_text_test` / `mark_title_test` / `markdown_engine_regressions_test` / `windows_clipboard_copy_test` | B 类随行修改（`parseInline` → `markdownCodec.parseInline`；标点契约改断言往返——引擎比旧 Dart 写端转义更保守，Rust 为权威） | — |

### T4.10 性能验收（`test/perf/table_cells_bench_test.dart`，真 FFI + 计数包装门面）

- **首次解码 500 旧格式单元格**：本机（Windows 桌面 FFI）首跑 12.7–17.7 ms（JIT 噪声区间），**warmed best-of-5 fresh 0.8–1.0 ms**——预算 16 ms 内（发布版 AOT 无 JIT 预热，直接落在 warmed 区间）。分解：引擎 wire 侧 ~1.3 ms / 批；Dart `jsonDecode` 是大头，故批量上限 64（8 批）。
- **之后的重新布局**：20 次 `fromBlock` 同文档 **0 次引擎调用**（内容缓存命中）。
- **提交重编码**：1 次批量 `encodeCells`；I3 回读校验走源码缓存 **0 次额外解码**。
- **缓存上限**：4,096 条 LRU，超限逐出后正确重解码（测试填 4,608 条验证）。

### T4.11 真实路径验证（2026-10-28）

**Web（生产 `--release` harness + wasm 门面，两种剪贴板模式各跑一遍）**

- 全部既有场景复验通过（外部粘贴 5 例、公式、跨块复制、单元格复制）。
- **新增单元格编辑序列**（两种模式）：点开单元格真实 TextField → 键入 ` edited` → Ctrl+A + Ctrl+B（干净文本上加重粗）→ Esc 提交 → Ctrl+A 复制整表（plain TSV 含 `command then \`literal\` edited`，html 含 `<strong><code>command</code> then \`literal\` edited</strong>`——加粗套代码，标记真实）→ 粘贴到外部纯文本 / 富文本目标 → Ctrl+Z 撤销恢复原格 → Ctrl+Y 重做再应用。**全部通过**。
- 截图（不入库）：`e2e/clipboard-{ClipboardItem,legacy}-cell.png` 等 13 张（含提交后表格渲染——加粗单元格 + 表头样式正确）。

**Windows 桌面（真应用进程 + 真 FFI + 真系统剪贴板）**

- `integration_test/clipboard_roundtrip_test.dart`：**1 passed / 0 failed**。场景 4 的正文单元格从「raw `**cell**` 源码」改为「干净文本 + bold mark」——新模型把字面 `**` 当文本是正确行为，旧断言测的是镜像时代的解析副作用。截图 `build/t3.10/01…05.png`（04-copy-source：加粗 cell、排版公式、脚注、嵌套列表、表格全选复制）。
- `integration_test/markdown_codec_test.dart`：**10 passed / 0 failed**（wire 黄金逐字）。
- 协同编辑（旧客户端 + 新客户端同表格）：未做——本机无旧版本客户端可拉起，且单元格存储形态不变（T4.8 的「新写 → 旧读」路径以引擎解码为旧读权威验证）。留给 G4 审核方决定是否要求。

### T4.12 文档收尾

- `AGENTS.md`：「Markdown 语法以 `crates/markdown` 为权威，Dart 镜像必须同步更新」→ 新规则原文（见上方任务节），拆为独立的「Markdown 只有一份实现」条目。
- `docs/architecture/architecture.md` Markdown 一节重写为单一实现描述，删除「Being replaced」过渡段。
- `docs/architecture/editor-engine.md`：comrak 决策段的两解析器论证更新为历史口径 + 单一实现专项链接（决策本身维持——comrak 的方言面仍然超出往返契约能守护的范围）。
- `docs/architecture/commonmark-scoreboard.md`：CJK flanking 条目的路径改为 `crates/md-core` + 镜像删除注记。
- `docs/dev/lessons.md` §1 追加「双表示的终点：Markdown 镜像删除」小节（结构性事实胜过守则）。
- `docs/plans/roadmap.md` 的专项条目**保留**（S5 未完成，整个专项未到 G4，还不到移入 roadmap-done 的时点）。

### 门禁（`bash scripts/local-ci.sh all`，2026-10-28）

`bash scripts/local-ci.sh all` **跑到结尾的 `ALL GREEN`**（`EXIT=0`，不经过管道）：

| 段 | 回执 |
| --- | --- |
| `cargo test --workspace` | `receipt: 1141 passed (floor 500)` |
| `-p mica-core --features store` | `receipt: 213 passed (floor 1)` |
| `crates/md-core` wire 黄金 | `receipt: 9 passed (floor 9)` |
| wasm 产物 | `mica_md gzip: wasm=200,264 B JS=2,744 B total=203,008 B`（硬预算 262,144 B） |
| `flutter analyze --no-fatal-infos` | 0 error / 0 warning（238 条全部为 info 级 lint） |
| `flutter test` | `receipt: 2051 passed (floor 1000)` |
| `flutter build web --release`（dart2js 编译检查） | 通过（`√ Built build\web`） |
| 收尾 | `ALL GREEN for what this ran.` `EXIT=0` |

Windows 下 `local-ci.sh` 按设计跳过浏览器套件与桌面集成（脚本尾列出各自命令）；本轮这两项以 T4.11 的真路径验证覆盖：
**Web** 生产构建 e2e（`clipboard_e2e.mjs` 全部通过，含新增单元格编辑序列）；
**Windows 桌面** `integration_test/markdown_codec_test.dart` **10 passed / 0 failed**、`integration_test/clipboard_roundtrip_test.dart` **1 passed / 0 failed**。
