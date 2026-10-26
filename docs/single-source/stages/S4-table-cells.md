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

（执行方追加。）
