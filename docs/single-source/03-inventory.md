# 03 现状清单：每一行要动的代码在哪、去哪

> 生成于 2026-10-26，基于 main `5e9fda5d` 之后的代码。行号会随开发漂移，**以符号名为准**；
> 执行前用 `grep -n` 重新定位。表中「去向」是规范性的：执行 agent 偏离时必须在阶段执行记录里说明理由。

## 1. Rust：`mica_markdown` 的使用方（S1 必须让它们零改动）

| 使用方 | 用到的 API | 备注 |
| --- | --- | --- |
| `crates/app-core` | `documents.rs: pub use mica_markdown::*`（**整个 crate 重新导出**）、`DocumentSnapshotPayload`、`Block`、`document_title`、`import_markdown`、`properties::{parse_properties, PropertyValue}` | `pub use *` 意味着门面必须保留**每一个**现有 pub 项 |
| `crates/api-server` | `document_title`×8、`DocumentSnapshotPayload`、`import_markdown`、`import_markdown_fragment`、`export_markdown`、`set_document_title`、`export_html`、`mangled_latex_*`、`block_text_preview`、`block_text_matches`、`markdown_find_text`、`replace_block_text`、`SAFE_HTML_CSP`、`Block` | feature `render` 开 |
| `crates/interchange` | `import_markdown`×15、`with_page_title`、`export_markdown`、`set_document_title`、`strip_page_title`、`document_title`、`Block` | |
| `crates/mcp-server` | `latex_guard`、`mangled_latex_message`、`mangled_latex_collision` | |
| `crates/mica-core` | `block.rs: pub use mica_markdown::Block`、`import_markdown`；`tests/markdown_invariant.rs` | `Block` 必须是**同一个类型** |
| FFI `clients/mica_flutter/rust` | `with_page_title`、`export_markdown`、`import_markdown`、`document_title`、`DocumentSnapshotPayload`、`Block`、`set_image_srcs`、`render_mermaid_svg`、`export_markdown_with_assets`、`export_html_document`、`MermaidHostTheme` | feature `render` 开 |

`mica_markdown` 现有全部 pub 项（门面必须逐个保留，签名不变）：

```
mod properties { PropertyValue, Property, parse_properties, upsert_property, remove_property }
struct DocumentSnapshotPayload; struct Block (+ new/with_text/with_data/with_children)
enum DocumentOperationError; type DocumentOperationResult
fn payload_from_value, export_markdown, document_title, set_document_title, with_page_title, strip_page_title,
   export_markdown_with_assets, export_html, export_html_document, set_image_srcs,
   render_mermaid_svg_with_id (render / 非 render 两个版本), render_mermaid_svg,
   import_markdown, import_markdown_fragment, block_index, is_descendant
const SAFE_HTML_CSP; struct MermaidHostTheme
pub use text_edit::{block_text_matches, block_text_preview, markdown_find_text, replace_block_text}
pub use inline::math_run_spans
pub use latex_guard::{JSON_ESCAPE_COLLISIONS, mangled_latex_collision, mangled_latex_message}
```

`crates/markdown/src` 模块（共 9,402 行）→ S1 去向：

| 模块 | 行数 | 去向 |
| --- | --- | --- |
| `inline.rs` | 2,205 | `md-core/src/parse/inline.rs` |
| `block_handlers.rs` | 1,535 | `md-core/src/parse/block_handlers.rs` |
| `block.rs` | 1,223 | `md-core/src/parse/block.rs`（`push_block` 改用注入的 `BlockIds`，见 S1 T1.4） |
| `lib.rs` | 1,417 | 拆：模型 → `md-core/model.rs`；import/export/html 入口 → `md-core/api.rs`；渲染 → `md-core/render.rs`；页面标题、`export_html_document`、`SAFE_HTML_CSP`、`DocumentOperationError` 等留在门面；四个 `#[cfg(test)] mod` 跟着被测代码走 |
| `html.rs` | 1,016 | `md-core/src/html/html.rs` |
| `properties.rs` | 592 | `md-core/src/properties.rs`（门面 `pub use mica_md::properties`） |
| `export_md.rs` | 551 | `md-core/src/serialize/export_md.rs` |
| `entities.rs` | 211 | `md-core/src/parse/entities.rs` |
| `text_edit.rs` | 203 | **留在门面**（MCP 编辑辅助；它用到的 `decode_table_cell` 改用公开的 `mica_md::decode_cell`） |
| `span_walk.rs` | 192 | `md-core/src/serialize/span_walk.rs` |
| `latex_guard.rs` | 134 | **留在门面**（MCP 防护） |
| `scan_memo.rs` | 123 | `md-core/src/parse/scan_memo.rs` |

## 2. Dart：镜像代码逐项去向

图例：**S3-删** = S3 删除（由门面取代）；**S4-删** = S4 删除；**留** = 编辑器自己的模型/交互，不是镜像；
**迁** = 搬到新位置，行为不变；**门面** = 改为调用 `markdownCodec`。

### 2.1 `lib/editor/markdown.dart`（1,492 行）

| 行（约） | 符号 | 外部使用者 | 去向 |
| --- | --- | --- | --- |
| 13–35 | `typedef BlockSpec` | `ai_edits.dart`、`editor.dart` | **迁** → `lib/markdown/markdown_codec.dart`；`markdown.dart` 删除后，两个使用者改 import |
| 326–364 | `retagMislabeledFences` | `controller.dart`、`editor.dart`、`highlight.dart` | **迁** → `lib/editor/paste_repair.dart`（粘贴启发式，D7：Dart 保留） |
| 365–434 | `unwrapNestedFences` | `editor.dart` | **迁** → `paste_repair.dart`（同上） |
| 219–239 | `isThematicBreak` | `table.dart` | **S3-删**（随 `looksLikeGfmTable` 一起消失） |
| 435–1347 | `markdownToBlocks` | `editor.dart`×5、`editor_ai.dart`、`main_shell.dart` | **S3-删** → `markdownCodec.parseBlocks` |
| 其余（`htmlBlockStart`、`setextLevel`、`deindentColumns`、`_collectRefDefinitions` 等） | 只被 `markdownToBlocks` 用 | — | **S3-删** |

S3 结束时 `lib/editor/markdown.dart` **整个文件删除**。

### 2.2 `lib/editor/marks.dart`（2,689 行）

| 行（约） | 符号 | 去向 |
| --- | --- | --- |
| 8–37 | `class Mark` | **迁** → `lib/markdown/mark.dart`；`marks.dart` 写 `export '../markdown/mark.dart';`（48 个 import 方零改动） |
| 38–292 | `marksFromData`、`marksToJson`、`buildMarkedSpan`、`rangeHasMark`、`mathRunAt`、`snapOutOfMathRun`、`mathRunEndingAt`、`mathRunStartingAt`、`applyMark`、`shiftMarks`、`splitMarks`、`concatMarks`、`_normalize` | **留**（编辑器标记模型与渲染） |
| 349–1388 | `normalizeLabel`、`_inlineSpanEnd`、`_canonicalizeBreaks`、`matchingBracket`、`bracketNesting`、`_ScanMemo`、`isUnicodeWhitespace`、`_codeSpanClose*`、`_findMathCloser*`、`extendedAutolink`、`_trimAutolinkEnd`、`_validAutolinkDomain`、`inlineHtmlEnd`、`parseEntity` | **S4-删**（行内解析器内部） |
| 869–921 | `parseInlineMath` | **门面**（S3）→ 新增 `markdownCodec.parseInlineMath`（Rust 侧新增 `parse_inline_math`，见 S1 T1.5）；然后 **S4-删** |
| 922–963 | `mathRunSpans` | **门面**（S3，`html_to_markdown.dart` 是唯一使用者）→ `markdownCodec.mathRunSpans`；然后 **S4-删** |
| 1389–1565 | `unescapeMd`、`_unescapeMd`、`autolinkTarget`、`_labelHasLinkCached`、`_LabelCache` | **S4-删** |
| 1566–1950 | `parseInline` 及内部 | **S4-删**（S3 期间仍被表格路径使用，S4 删除最后几处调用后删除） |
| 1930–2118 | `_Delim`、`_isWordChar`、`_isMdPunct`、`_isCjk`、`_processEmphasis` | **S4-删** |
| 2119–2238 | `escapeBlockLeader(s)`、`escapeMarkdownTitle`、`escapeMarkdownDestination`、`escapeInline`、`inlineToMarkdown` | **S3-删**（使用者 `controller.dart` 的 `_rangeMarkdown` 在 S3 删除）；`inlineToMarkdown` 的表格/单元格使用者在 S4 删除后 **S4-删** |
| 2239–2562 | `_rendersTerminal`、`_RenderMark`、`_sortedForRender`、`_startBoundBelow`、`_ActiveMarks`、`_RenderLevel`、`_renderSpan`（Markdown 序列化的嵌套渲染） | 随 `inlineToMarkdown` **S4-删** |
| 2563–2682 | `escapeHtml`、`escapeHtmlAttr`、`inlineToHtml`、`_renderSpanHtml` | **S3-删**（剪贴板 HTML 改由 `renderClipboardHtml` 产出）。若 S3 结束时仍有使用者，列入执行记录并在 S4 删 |
| 2683–2689 | `linkClickHit` | **留** |

S4 结束时 `marks.dart` 预计约 300 行（只剩标记模型、渲染、`linkClickHit`）。

### 2.3 `lib/editor/controller.dart` 中的序列化（约 1,085–1,822 行）

| 符号 | 用途 | 去向 |
| --- | --- | --- |
| `selectionClipboardText` | 复制的 text/plain（字面文本） | **留**；其中表格单元格的 `parseInline(cell).text` 在 S3 改为 `markdownCodec.decodeCells`，S4 改为 `cell.text` |
| `selectionText` / `documentMarkdown` / `_rangeMarkdown` / `_blockPrefix` / `_continuationPrefix` / `_codeFenceFor` | 选区 / 整篇 → Markdown（复制的 Markdown flavor、AI 输入、AI 读当前页） | **S3**：改成「切片成 `List<BlockSpec>` → `markdownCodec.serializeBlocks`」；`_rangeMarkdown` 及三个辅助函数删除 |
| `_selectionSlices` | 选区切片 | **留**（S3 复用它构造 `BlockSpec` 片段） |
| `selectionHtml` / `_tableHtml` | 复制的 text/html | **S3**：改为 `markdownCodec.renderClipboardHtml`；两个函数体删除 |
| `previewTableCell` / `setTableCell` | 单元格写入 | **S4**：参数从 `String`（源码）改为 `InlineText` |

### 2.4 其他 Dart 文件

| 文件 | 符号 | 去向 |
| --- | --- | --- |
| `lib/editor/list_serialization.dart`（49 行） | `ListSerialization`、`markdownListLayout` | **S3-删**（只被 `_rangeMarkdown` 用；Rust `export_md` 自己处理列表编号） |
| `lib/editor/table.dart`（301 行） | `TableData` | **S4 改**：`rows` 从 `List<List<String>>` 改为 `List<List<TableCell>>`（值相等的不可变类，见 S4 T4.1）；`_cellText` / `_encodeCell` 删除，改走 `markdownCodec.decodeCells` / `encodeCells`（批量） |
| | `looksLikeGfmTable`、`_startsBlock`、`_splitRow` | **S3-删**（只被 `markdownToBlocks` 用） |
| | `tableToMarkdown` | **S3-删**（只被 `_rangeMarkdown` 用） |
| `lib/editor/cell_edit_controller.dart`（167 行） | `CellEditController(String rawMarkdown)`、`serialize()`、`selectionHtml()` | **S4 改**：构造参数改为 `InlineText`，删除 `serialize()`；`selectionHtml()` 在 S3 改为 `renderClipboardHtml` |
| `lib/editor/render.dart` | `cellDisplaySpan(String raw, …)` 里的 `parseInline(raw)` | **S4 改**：参数改为 `InlineText`，直接 `buildMarkedSpan` |
| `lib/editor/editor.dart` | `markdownToBlocks`×5（约 1268、1297、1787、5664、5756 行） | **S3 门面** `parseBlocks` |
| | `parseInlineMath`（约 1232 行） | **S3 门面** |
| | 表格复制 HTML `parseInline` + `inlineToHtml`（约 4105 行） | **S3 门面** `renderClipboardHtml` |
| | `unwrapNestedFences` / `retagMislabeledFences`（约 1150 行） | **留**，改 import `paste_repair.dart` |
| `lib/editor/editor_ai.dart` | `markdownToBlocks`（约 383 行） | **S3 门面** |
| `lib/main_shell.dart` | `markdownToBlocks`（`_aiCurrentFromMarkdown`，约 2987 行） | **S3 门面**（`main.dart` 库行数棘轮：净变化 ≤ 0） |
| `lib/editor/html_to_markdown.dart`（782 行） | `htmlToMarkdown` | **留**（HTML → Markdown 转换，Rust 无对应物，不是镜像；本专项不搬）；它用的 `mathRunSpans` 在 S3 改走门面 |
| `lib/editor/ai_edits.dart` | `BlockSpec` import | S3 改 import |
| `lib/editor/controller.dart` `applyInputRules` | 输入快捷规则 | **留**（编辑器交互） |

## 3. Dart 测试：35 个依赖镜像的文件怎么处理

规则：
- **A 类（语法语义测试）**：测的是「某段 Markdown 解析成什么 / 序列化成什么」。处理：在 `crates/md-core/tests/` 或 `crates/markdown/tests/`
  **确认有等价的 Rust 测试**（没有就新增，迁移用例），然后删除 Dart 测试。对每个文件在执行记录里写「Rust 对应测试：文件::函数」。
- **B 类（编辑器行为测试）**：测的是「在编辑器里粘贴 / 复制 / 编辑后得到什么」。处理：保留，`setUpAll(useNativeMarkdownCodec)`，
  断言改为对门面结果的断言（Rust 行为为准；与原断言不同处按 D7 处理并记录）。
- **C 类（对拍测试）**：`markdown_conformance_test`、`markdown_export_conformance_test`、`render_span_equivalence_test` 等两引擎对拍。
  处理：S3/S4 完成后对拍对象已不存在 → 改造成「门面 ⇄ 黄金文件」的**绑定契约测试**（证明 JSON 映射没丢东西），保留一份即可。

初步分类（执行者核对后在 S3/S4 执行记录里给出最终表）：

| 文件 | 用例数 | 初判 | 处理阶段 |
| --- | --- | --- | --- |
| `markdown_conformance_test.dart` | 2 | C | S3 |
| `markdown_export_conformance_test.dart` | 2 | C | S3 |
| `render_span_equivalence_test.dart` | 2 | C | S4 |
| `link_parity_test.dart` | 3 | A | S3 |
| `math_code_span_test.dart` | 5 | A（自述「Dart mirror of crates/markdown/tests/math_code_span.rs」） | S4 |
| `image_alt_export_test.dart` | 3 | A（自述「Mirrors crates/markdown/tests/image_alt_export.rs」） | S3 |
| `escapes_autolinks_test.dart` | 7 | A | S4 |
| `cjk_emphasis_and_fence_test.dart` | 12 | A（+ 粘贴围栏启发式部分是 B） | S3/S4 拆分 |
| `footnotes_test.dart` | 7 | A | S3 |
| `front_matter_test.dart` | 7 | B（「粘贴时丢掉元数据」）+ A | S3 |
| `inline_scan_perf_test.dart` | 9 | A（性能：Rust 已有 `inline_scan_perf.rs`） | S4 |
| `nested_bracket_perf_test.dart` | 4 | A（Rust 已有 `nested_bracket_perf.rs`） | S4 |
| `linked_code_span_test.dart` | 5 | A（Rust 已有 `linked_code_span.rs`） | S4 |
| `markdown_engine_regressions_test.dart` | 9 | A | S3/S4 |
| `markdown_image_test.dart` | 6 | A | S3 |
| `tilde_range_test.dart` | 4 | A（Rust 已有 `tilde_runs.rs`） | S4 |
| `table_inline_code_test.dart` | 3 | A（Rust 已有 `table_inline_code.rs`）+ 单元格模型 | S4 |
| `numbered_continuation_test.dart` | 1 | A | S3 |
| `list_serialization_test.dart` | 3 | A（`markdownListLayout` 被删） | S3 |
| `url_shaped_link_label_test.dart` | 4 | B（复制链接再粘贴） | S3 |
| `paste_audit_test.dart` | 24 | B | S3 |
| `paste_block_kind_test.dart` | 12 | B | S3 |
| `paste_formula_test.dart` | 13 | B（`parseInlineMath`） | S3 |
| `paste_inline_marks_test.dart` | 23 | B | S3 |
| `paste_structure_test.dart` | 17 | B | S3 |
| `paste_wrapped_list_test.dart` | 2 | B | S3 |
| `html_paste_math_test.dart` | 10 | B（`htmlToMarkdown` + 门面） | S3 |
| `copy_markdown_test.dart` | 12 | B | S3 |
| `copy_code_language_test.dart` | 8 | B | S3 |
| `clipboard_flavor_roundtrip_test.dart` | 3 | B | S3 |
| `clipboard_plain_text_test.dart` | 6 | B | S3（S4 再改单元格部分） |
| `ai_document_markdown_test.dart` | 2 | B | S3 |
| `cell_copy_marks_test.dart` | 1 | B | S4 |
| `cell_edit_controller_test.dart` | 10 | B（单元格模型） | S4 |
| `atomic_block_select_test.dart`、`divider_test.dart`、`list_indent_test.dart`、`depth_clamp_test.dart`、`mark_title_test.dart`、`table_alignment_preservation_test.dart`、`windows_clipboard_copy_test.dart` | — | B（少量用到 `markdownToBlocks` 造数据 / `selectionText`） | S3 |

**禁止**：为了让 B 类测试不依赖原生库而写一个纯 Dart 的假门面——那就是第三份实现。

## 4. 构建与 CI 接入点（S2 必须全部接上 wasm 构建）

| 位置 | 现状 | S2 要做 |
| --- | --- | --- |
| `justfile` `dev-web`（约 78 行）、`build-web`（约 216 行） | `flutter build web --release --no-web-resources-cdn` | 先跑 `bash scripts/build-md-wasm.sh` |
| `scripts/local-ci.sh`（flutter 部分，约 83 行） | 同上 | 同上；`browser-tests.sh` 之前也要构建 |
| `scripts/browser-tests.sh` | 浏览器套件 | 开头构建 wasm（产物复制到 `test/support/mica_md/`） |
| `scripts/release-images.sh`（约 26 行） | web bundle | 同上 |
| `.github/workflows/ci.yml`（约 333、419、537 行三处 build web）、浏览器套件步骤 | — | 每处 build web 之前：setup-rust（已有 action）+ `rustup target add wasm32-unknown-unknown`（由 `rust-toolchain.toml` 自动）+ 安装钉版本的 `wasm-bindgen-cli` + 构建脚本 |
| `.github/workflows/release.yml`（约 458 行） | web build | 同上 |
| `scripts/test-ci-paths.sh` + `ci.yml` 的 paths 作业 | 按路径选作业 | `crates/md-core/**`、`crates/md-wasm/**` 的改动必须触发 Rust、Flutter、web 作业 |
| `scripts/check-ignored-sources.sh` | 拒绝被忽略的源码 | `clients/mica_flutter/web/mica_md/`、`clients/mica_flutter/test/support/mica_md/` 加进允许名单（生成物） |
| `.gitignore` | — | 忽略上面两个目录 |
| `deploy/nginx.conf` | 已有 `application/wasm` gzip | 无需改 |
| `rust-toolchain.toml` | 无 targets | `targets = ["wasm32-unknown-unknown"]` |
| `AGENTS.md` 依赖豁免清单 | — | 加 `wasm-bindgen`（Rust）/ `wasm-bindgen-cli`（构建工具）：仅 Web、仅 `crates/md-wasm` |

## 5. 专项结束时要改的文档

| 文档 | 改动 |
| --- | --- |
| `AGENTS.md`「Rust-first 数据面」一条 | 「Markdown 语法以 `crates/markdown` 为权威，Dart 镜像必须同步更新」→「Markdown 只有 Rust 一份实现（`crates/md-core`），Dart 只通过 `MarkdownCodec` 门面调用；禁止在 Dart 里新增 Markdown 语法逻辑」 |
| `docs/architecture/architecture.md` Markdown 一节（约 115–127 行） | 改为单一实现的描述，链接本专项 |
| `docs/architecture/editor-engine.md` | 「`markdown/`」模块描述、粘贴/复制路径 |
| `docs/architecture/commonmark-scoreboard.md` | 路径从 `crates/markdown` 改到 `crates/md-core`（如测试文件搬家） |
| `docs/dev/lessons.md` §1 | 补一段「双表示的终点：Markdown 镜像删除」 |
| `docs/plans/roadmap.md` | 本专项条目移入 `roadmap-done.md` |
