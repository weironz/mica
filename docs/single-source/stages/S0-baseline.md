# S0 基线与安全网（2–3 天，无产品行为变化）

## 目标

在删任何 Dart 代码之前，准备好两样东西：

1. **差分对拍工具**：同一批输入同时交给 Dart 镜像和 Rust 引擎，列出所有不一致。它是 S3/S4 的安全网——
   每一条差异都是「切换后用户会看到的行为变化」，必须逐条分类处理（D7），不能带着未知差异删 Dart 代码。
2. **性能基线**：现在的 Dart 镜像有多快。S2/S3/S4 的性能预算（02 §7）都是相对它的。

## 前置

无。可与 S1 同时开始（S0 只读现有代码、只加测试和工具）。

## 任务

### T0.1 语料库清单 `test/support/markdown_corpus.dart`（Dart，VM）

一个函数返回全部语料，每条带来源标签（报告里要能定位）：

```dart
/// Every Markdown input the differential and the benchmarks run on. Paths are
/// relative to clients/mica_flutter (where `flutter test` runs).
List<({String id, String markdown})> markdownCorpus({int fuzzCount = 5000, int seed = 20261026}) {
  // 1. ../../crates/markdown/tests/fixtures/commonmark-spec-0.31.2.json   652 例，id = 'spec:<example>'
  // 2. ../../crates/markdown/tests/fixtures/gfm-extensions.json           24 例， id = 'gfm:<i>'
  // 3. ../../crates/markdown/tests/fixtures/golden-master.json            742 例，id = 'golden:<key>'
  // 4. ../../crates/markdown/tests/fixtures/conformance/*.md              105 个，id = 'conf:<file>'
  // 5. ../../crates/markdown/tests/fixtures/paste.md、tests/data/*.md      id = 'file:<name>'
  // 6. 生成的模糊输入：从下面的「词元」里按 seed 随机拼接 1–40 个，id = 'fuzz:<n>'
  //    词元至少覆盖：'**' '*' '_' '__' '~~' '~' '`' '```' '~~~' '$' '$$' '\\(' '\\)' '[' ']' '(' ')' '![' '<' '>' 'http://x.y'
  //    '# ' '## ' '- ' '* ' '1. ' '2) ' '> ' '>> ' '| a | b |' '|---|---|' '---' '***' '[^1]' '[^1]: ' '[x]: /u'
  //    '\n' '\n\n' '  \n' '\\\n' '\t' '    ' '中文' '😀' '𠀀' 'é' '&amp;' '&#123;' '\\*' '\\' 'a' 'b c'
}
```

固定 seed，保证报告可复现。`fuzzCount` 在 CI 用 5,000；本地调试可以调大。

### T0.2 差分对拍 `test/markdown_differential_test.dart`（Dart，VM，需要原生库）

S0 阶段 Rust 侧**只能通过现有 FFI**取结果（S2 的新接口还没有）：

| 比较项 | Dart 侧 | Rust 侧（S0，现有 FFI） | Rust 侧（S2 起改用） |
| --- | --- | --- | --- |
| 块解析 | `markdownToBlocks(md)` | `MicaDocument.fromMarkdown(markdown: md).toBlocksJson()`，取根块 children | `markdownCodec.parseBlocks(md)` |
| Markdown 序列化 | 用 Dart 解析结果造节点 → `EditorController(...).documentMarkdown()` | `MicaDocument.fromBlocksJson(...)`（同一批块）`.exportMarkdown()` | `markdownCodec.serializeBlocks(...)` |
| 往返 | Dart：`markdownToBlocks(documentMarkdown(markdownToBlocks(md)))` 是否稳定 | Rust 同理 | 同左 |

> S0 经过 `MicaDocument` 走了一趟 yrs，标记会被规范化（排序、合并相邻同类标记）。比较前两边都按 02 §3.4 规范化；
> 若仍出现只因 CRDT 往返产生的差异，在报告里标为 `crdt-normalization`，S2 切到 `parseBlocks` 后应消失。

规范化后逐条比较 `kind`、`text`、`data`（深比较）。输出：

- `build/differential/report.json`：`[{id, category, field, dart, rust}]`
- `build/differential/summary.md`：按类别计数、每类前 10 例

类别（`category`）由比较函数自动判定，至少区分：`kind`（块类型不同）、`text`（文本不同）、`marks`（只差标记）、
`data:<key>`（只差某个 data 键）、`block-count`（块数不同）、`export`（序列化不同）、`roundtrip`（某一侧往返不稳定）、
`crash`（某一侧抛异常）。

**棘轮**：`test/differential_baseline.json` 记录 S0 当时每个类别的差异数和全部差异的 id。测试断言：
当前差异 ⊆ 基线（**只许减不许增**）。S0 提交基线本身；之后任何引擎改动导致新增差异，测试就会失败。

### T0.3 性能基线 `test/perf/markdown_baseline_bench_test.dart`

在 VM（`flutter test`）和浏览器（`flutter test --platform chrome`，即 dart2js）各跑一遍。每项运行 7 次取中位数，
前 2 次丢弃作为预热。

| 项 | 输入 |
| --- | --- |
| `parse.87` | 87 字符单段，含 `**粗体**`、`` `code` ``、链接（与 7 月 spike 同形） |
| `parse.10k` | 拼接 conformance fixtures 到约 1 万字符 |
| `parse.100k` | 同上到约 10 万字符 |
| `inline.cell×500` | 500 个单元格源码（从 fixtures 的表格与段落里取行内片段），逐个 `parseInline` |
| `serialize.10k` | `parse.10k` 的结果 → `documentMarkdown()` |
| `html.10k` | 同上 → 全选 `selectionHtml()` |

输出一行 JSON：`{"platform":"vm|chrome","item":"parse.10k","median_ms":…}`。把结果抄进本文末尾的「执行记录」，并提交到
`docs/single-source/stages/S0-baseline.md`（这是 02 §7 预算的基准）。

Rust 侧基线：新增 `crates/markdown/examples/md_bench.rs`（`cargo run -p mica-markdown --release --example md_bench`），
用 `std::time::Instant` 测 `import_markdown` / `export_markdown` / `export_html` 对同样三档输入的耗时。**不引入 criterion**。

### T0.4 启动基线

- 桌面（Windows 发布版）：在现有启动诊断里记录 `main()` 开始到 `runApp` 的耗时（若已有就直接读取），记 5 次中位数。
- Web：Playwright 打开生产构建，记录 `performance.now()` 到第一帧（`flutter-first-frame` 事件）的时间，5 次中位数。

### T0.5 剪贴板输出快照

为 S3 的「复制结果变化报告」准备：从 conformance fixtures 选 30 个，导入编辑器后全选，
记录 `selectionText()`、`selectionHtml()`、`selectionClipboardText()` 三个结果到 `build/differential/clipboard_baseline.json`（不提交，S3 时现场生成对比）。
在本文写清生成命令，保证 S3 执行者能在**切换前**的提交上重新生成。

## 验收

- [ ] `flutter test test/markdown_differential_test.dart` 通过，`differential_baseline.json` 已提交，`summary.md` 的分类计数抄进执行记录。
- [ ] 两个平台的基准数字、Rust 基准数字、启动基线都在执行记录里。
- [ ] 本阶段没有任何 `lib/` 或 `crates/*/src` 的改动（只加测试、示例、工具）。
- [ ] 门禁 `bash scripts/local-ci.sh all` 全绿，回执贴进执行记录。

## 执行记录

（执行方追加：日期、提交号、对拍分类计数、基准数字表、偏离方案之处。）
