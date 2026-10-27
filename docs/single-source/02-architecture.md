# 02 目标架构与契约

> 本文是**契约**。§2（Rust 公共 API）、§3（线格式）、§5（Dart 门面接口）**已冻结（2026-10-27，`single-source` @ `f21daf18`，G1 复核通过，见 [reviews.md](reviews.md)）**。
> 冻结后改动须在 PR 里单列「契约变更」并经审核方同意。§6 的 Schema 是规范性的，代码与它不一致视为 bug。

## 1. 分层与依赖

### 1.1 crate / 包结构（S1–S2 完成后）

```
crates/
  md-core/        package `mica-md`        lib `mica_md`        ← SDK 核心（新）
    src/
      lib.rs          公共 API 再导出 + crate 文档
      model.rs        Block / Document(=DocumentSnapshotPayload) / Mark / InlineText / SCHEMA_VERSION / MdError
      schema.rs       kinds / marks / data_keys 常量表（与 §6 一致，由测试校验）
      ids.rs          BlockIds（Uuid | Sequential）
      api.rs          parse / parse_body / parse_inline / serialize / serialize_body / serialize_inline /
                      render_html / render_html_body / decode_cell / encode_cell
      wire.rs         跨语言 JSON 线格式（§3），FFI 与 wasm 共用，保证两端逐字节一致（HTML Export 例外，见 §3.1）
      parse/          block.rs  block_handlers.rs  inline.rs  scan_memo.rs  entities.rs   （自 crates/markdown 迁入）
      serialize/      export_md.rs  span_walk.rs                                          （迁入）
      html/           html.rs                                                              （迁入）
      render.rs       feature "render"：数学 → SVG（ratex）、mermaid → SVG（merman）、MermaidHostTheme
      properties.rs   front matter 属性（迁入，保持 `pub mod properties`）
    tests/            新增：public_api.rs、schema_sync.rs、wire.rs、cell_roundtrip.rs（S1/S2）
  markdown/       package `mica-markdown`  lib `mica_markdown`  ← Mica 适配 + 兼容门面（保留，变薄）
    src/lib.rs        `pub use mica_md::{…}` 再导出 + Mica 专属：页面标题、export_html_document、
                      SAFE_HTML_CSP、DocumentOperationError、payload_from_value、block_index、is_descendant
    src/text_edit.rs  MCP 文本查找替换（保留在适配层）
    src/latex_guard.rs MCP JSON 转义防护（保留在适配层）
    tests/            现有全部集成测试原地不动（S1 证明行为不变的依据）
  md-wasm/        package `mica-md-wasm`   lib `mica_md_wasm`  cdylib  ← Web 绑定（新，S2）
clients/mica_flutter/
  rust/src/api/markdown.rs                 ← 桌面 FFI 绑定（新，S2，frb 生成 Dart）
  lib/markdown/                            ← Dart 门面（新，S2）
    markdown_codec.dart        接口 + 全局访问器 + 初始化 + 条件导入
    markdown_codec_io.dart     桌面实现（调用 frb 生成的 `api/markdown.dart`）
    markdown_codec_web.dart    Web 实现（dart:js_interop 调用 wasm-bindgen 导出）
    markdown_wire.dart         JSON ⇄ Dart 类型（纯 Dart，两端共用）
    mark.dart                  `class Mark`（自 editor/marks.dart 迁入；marks.dart re-export，调用点零改动）
  web/mica_md/                 wasm 产物（构建生成，**不提交**，.gitignore）
```

依赖方向（不允许反向）：

```
mica-md  ←  mica-markdown  ←  mica-core / app-core / interchange / mcp-server / api-server / FFI crate
mica-md  ←  mica-md-wasm
mica-md  ←  FFI crate（rust/src/api/markdown.rs 直接用 mica_md，不经 mica-markdown）
```

`mica-md` 的依赖只允许：`serde`、`serde_json`、`thiserror`；`uuid`（feature `uuid`，默认开）；
`merman` / `ratex-*`（feature `render`，默认关）。**不允许**引入其他依赖（AGENTS.md 依赖原则）。

### 1.2 为什么不新增「中立 AST」层（D4）

讨论时曾设想「Markdown ⇄ 中立语法树 ⇄ Mica 块」三层。代码调研后否决：

1. 现有解析器**直接产出块**，中间没有树；插一层树意味着重写解析器输出端、所有导出端，并让每次转换多走一次。
2. 扁平块 + 标记（UTF-16 偏移）本身就是块编辑器通用的模型（ProseMirror/Quill/BlockSuite 都是结构化模型），
   SDK 的卖点恰恰是「Markdown ⇄ 块文档、往返一致」，不是「又一个 CommonMark AST」。
3. 两层表示是本仓库头号教训（`docs/dev/lessons.md` §1「双表示」）。

所以 SDK 的公共模型 = 现有 `Block` / `DocumentSnapshotPayload`，补上**规范化的 Schema 文档**（§6）和**类型化视图**
（`Mark`、`InlineText`）。Mica 专属约定（根块 `data.title`、`file_id` 资产、`mica://page/` 链接）放在适配层或文档中标明。

## 2. Rust 公共 API（`mica_md`，S1 交付并冻结）

```rust
// ---------- model.rs ----------
pub const SCHEMA_VERSION: i32 = 1;

/// 文档：一个根块 + 扁平块表。字段名、serde 形状与现有 `DocumentSnapshotPayload` 完全一致（同一个类型）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentSnapshotPayload { pub schema_version: i32, pub root_block_id: String, pub blocks: Vec<Block> }
pub type Document = DocumentSnapshotPayload;

/// 与现有 `mica_markdown::Block` 是同一个类型（迁移，不复制）；`Block::new/with_text/with_data/with_children` 保留。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
  pub id: String,
  #[serde(rename = "type")] pub kind: String,
  #[serde(default)] pub text: String,
  #[serde(default, skip_serializing_if = "Value::is_null")] pub data: Value,
  #[serde(default)] pub children: Vec<String>,
}

/// 行内标记，`[start, end)`，**UTF-16 code unit** 偏移（与 Dart / JS 字符串一致）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
  pub start: usize,
  pub end: usize,
  #[serde(rename = "type")] pub kind: String,
  #[serde(default, skip_serializing_if = "Option::is_none")] pub href: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")] pub title: Option<String>,
}

/// 纯文本 + 标记；`marks` 按 (start, end, kind) 升序（规范化顺序，见 §3.4）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InlineText { pub text: String, pub marks: Vec<Mark> }

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MdError {
  #[error("unsupported document schema version: {0}")] UnsupportedSchemaVersion(i32),
  #[error("block not found: {0}")] BlockNotFound(String),
  #[error("invalid input: {0}")] InvalidInput(String),   // 仅 wire.rs 使用
}

// ---------- ids.rs ----------
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockIds {
  /// `block_<uuid-v4-simple>`，现有行为。需要 feature `uuid`（默认开）。
  #[cfg(feature = "uuid")] Uuid,
  /// `<prefix><n>`，n 从 0 递增。确定性，wasm 与测试用；不需要随机数。
  /// 例外（G1 裁决 #5）：调用方的 `root_id` 恰好落在 `<prefix><n>` 命名空间（如 `"b0"` + `"b"`）时，
  /// 前缀确定性地改为 `<prefix>_`，n 仍从 0 起，保证 id 唯一。正常输入不受影响。
  Sequential { prefix: String },
}

// ---------- api.rs ----------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseMode {
  /// 整篇文档：开头的 `---` 围栏是 front matter，存进根块 `data.front_matter`。现有 `import_markdown`。
  Document,
  /// 片段：开头的 `---` 永远是分隔线。现有 `import_markdown_fragment`（MCP 追加/插入用）。
  Fragment,
}

#[derive(Debug, Clone)]
pub struct ParseOptions { pub mode: ParseMode, pub root_id: String, pub ids: BlockIds }
impl Default for ParseOptions { /* mode: Document, root_id: "root", ids: Uuid (feature uuid) 否则 Sequential{"b"} */ }

pub fn parse(markdown: &str, options: &ParseOptions) -> Document;
/// 只要正文：根块 children 顺序的块（`children` 为空，id 为 Sequential{"b"}）；front matter 丢弃。
/// 粘贴、AI 插入用。等价于 parse(Document 或 Fragment) 后取根块子块。
pub fn parse_body(markdown: &str, mode: ParseMode) -> Vec<Block>;
pub fn parse_inline(source: &str) -> InlineText;

/// 表格单元格存储形态 ⇄ InlineText。见 §6.4。
pub fn decode_cell(cell: &Value) -> InlineText;
pub fn encode_cell(cell: &InlineText) -> Value;

#[derive(Debug, Clone, Default)]
pub struct SerializeOptions {
  /// file_id → 写进 Markdown 的图片地址（相对路径或 URL）。现有 `export_markdown_with_assets` 的 `images`。
  pub assets: BTreeMap<String, String>,
}
pub fn serialize(doc: &Document, options: &SerializeOptions) -> Result<String, MdError>;
/// 片段：把 `blocks` 当作一个合成根块的全部子块序列化（复制选区、AI 输入用）。不输出 front matter。
pub fn serialize_body(blocks: &[Block], options: &SerializeOptions) -> String;
pub fn serialize_inline(text: &str, marks: &[Mark]) -> String;

/// HTML 的两种「方言」。它们服务不同的读者，不能合并（见 §2.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HtmlProfile {
  /// 导出 / 分享页：现有 `export_html` 的输出。feature `render` 下 mermaid → 内联 SVG、数学 → MathML。
  #[default] Export,
  /// 剪贴板 text/html：移植自 Dart `EditorController.selectionHtml` 的「Mica 剪贴板方言」——
  /// 数学保留 LaTeX 源码（`<span data-mica-math="1">$$…$$</span>`）、脚注定义 `data-mica-fndef`、
  /// 脚注引用 `data-mica-footnote`、mermaid 保持代码块、**从不输出 SVG/MathML**。
  /// 读者是外部编辑器（Word、Docs、Typora）和 Mica 自己的 `htmlToMarkdown`（Dart，粘贴路径）。
  Clipboard,
}

#[derive(Debug, Clone, Default)]
pub struct HtmlOptions {
  /// file_id → `<img src>`（现有 `set_image_srcs` 的语义）。
  pub image_srcs: BTreeMap<String, String>,
  pub profile: HtmlProfile,
}
pub fn render_html(doc: &Document, options: &HtmlOptions) -> Result<String, MdError>;
/// 片段 HTML，两种 profile 共用同一套净化（tagfilter、safe_url、strip_unsafe_attrs）。
pub fn render_html_body(blocks: &[Block], options: &HtmlOptions) -> String;

/// `$…$` 与 `\(…\)` 数学区间。**注意：偏移是 `char`（Unicode 标量）下标**——现有语义，
/// `latex_guard.rs` 按 char 切片依赖它，不改。跨语言时由 `wire::math_run_spans_json` 换算成 UTF-16。
pub fn math_run_spans(source: &str) -> Vec<(usize, usize)>;
/// 只识别行内数学（`$…$`、`\(…\)`），去掉定界符、其余字符原样保留，公式区间变成 `math` 标记（UTF-16）。
/// 粘贴单行文本时用（取代 Dart `parseInlineMath`），基于 `math_run_spans`，S1 新增。
/// 以 `$$` 开头的区间不算行内数学（与 Dart `MathRunSpans` 一致），这条过滤只在本函数里做，
/// `math_run_spans` 本身不改（G1 裁决 #3：`latex_guard` 依赖它的现有边界）。
pub fn parse_inline_math(source: &str) -> InlineText;

pub mod properties;   // 现有 front matter 属性 API，原样迁入
pub mod schema;       // §6 常量
pub mod wire;         // §3
pub mod render;  // 现有 render_mermaid_svg* / MermaidHostTheme / 数学 SVG。模块本身**无条件存在**：
                 // `MermaidHostTheme` 与「非 render 时返回 None」的 `render_mermaid_svg*` 现在就不受特性控制（lib.rs:536/550/619），
                 // 只有真正调用 merman / ratex 的函数体按 `#[cfg(feature = "render")]` 切换——与现状一致
```

### 2.1 剪贴板方言为什么要单独一个 profile

Mica 内部「复制 → 粘贴」走的是 **text/html**：复制时写 HTML（现在是 Dart `selectionHtml`），粘贴时 Dart `htmlToMarkdown`
把它转回 Markdown 再解析。这套 HTML 带着 Mica 自己的约定（`data-mica-*` 属性、LaTeX 源码、代码语言），
`htmlToMarkdown` 专门认这些约定。导出用的 HTML（MathML、mermaid SVG）粘贴回来就变不回源码了。
所以剪贴板方言是**一个独立的契约**：写端在 S3 移植进 Rust（`HtmlProfile::Clipboard`），读端仍是 Dart `htmlToMarkdown`
（Rust 没有 HTML 解析器，本专项不搬）。这条跨语言契约由 S3 的「Mica 内复制粘贴往返」测试守护（S3 T3.8）。

**Dart 侧预处理**（在调引擎之前，由 Dart 填好块数据，引擎只照写）：代码块语言。编辑器里 `language` 为空或 `auto` 时，
复制要输出自动识别的语言（`controller.dart _copyLanguage` → `highlight.dart detectLanguage`，Dart 独有）。
构造 `BlockSpec` 时把识别结果写进 `data.language`（`plaintext` 则删掉该键）。Markdown 与 HTML 两种复制都适用。

### 2.2 不变量（每条都要有测试，S1 交付）

| 编号 | 不变量 | 测试 |
| --- | --- | --- |
| I1 | `serialize(parse(md))` 与现有 `export_markdown(import_markdown(md))` 逐字节相同（全部 fixtures + golden-master） | `crates/markdown/tests/*` 原样通过 |
| I2 | 往返：`parse(serialize(doc))` 的块 kind/text/data 与 `doc` 相同（现有 `fixtures_round_trip`） | 现有测试 |
| I3 | `decode_cell(encode_cell(x)) == x` 对任意 `x`（含字面 `*`、`|`、`\`、换行、空串、非 BMP 字符），**除外**：无标记文本里的裸 URL / 邮箱（读回时按 GFM 自动链接加上 `link`）与首尾空白（读回时去掉）。这两类是存储格式的现有行为（§6.4），S1 不改（G1 裁决 #4） | `md-core/tests/cell_roundtrip.rs`（proptest，仓库已有该 dev 依赖）；除外项由显式测试钉住现状 |
| I4 | `parse_inline(serialize_inline(t, m)) == (t, m)`（规范化后） | 同上 |
| I5 | `BlockIds::Sequential` 下 `parse` 结果确定（同输入同输出），且与 Uuid 模式除 id 外完全相同 | `md-core/tests/ids.rs` |
| I6 | `schema.rs` 的常量覆盖 fixtures 中出现的全部 kind / data key / mark type | `md-core/tests/schema_sync.rs` |
| I7 | CommonMark 641/641、GFM 24/24 不退步 | 现有 `commonmark_scoreboard.rs` |
| I8 | 线格式里一切偏移都是 UTF-16：含非 BMP 字符（emoji、𠀀）的输入，`math_run_spans_json` / 标记偏移与 Dart `String` 下标一致 | `md-core/tests/wire.rs` |

## 3. 跨语言线格式（`mica_md::wire`，FFI 与 wasm 共用）

### 3.1 原则

- 所有绑定函数**同步**、`String` 进 `String` 出（JSON）。Dart 早已以 JSON 与 Rust 交换块（`from_blocks_json` 等）。
- 线格式逻辑**只在 `wire.rs` 写一次**，FFI（`rust/src/api/markdown.rs`）与 wasm（`crates/md-wasm/src/lib.rs`）都是一行转发。
  这保证桌面与 Web 对同一输入的输出**逐字节相同**（S2 契约测试验证）。
  **唯一例外**（M2 裁决，2026-10-27）：`render_html_body_json` 的 `export` profile 随 `render` 特性变化——桌面 FFI 开着 `render`，
  公式 / Mermaid 输出 SVG；wasm 不开，输出源码 span / code。跨端逐字节只约束 `clipboard` profile。
  Dart 门面因此**不暴露** Export HTML（`MarkdownCodec` 只有 `renderClipboardHtml`）；页面 HTML 导出在桌面走 `MicaDocument.exportHtml`、
  在 Web 走服务端，都不经过 wire。以后谁要在 Dart 侧加 Export 入口，先提契约变更。
- 错误：`Result<String, MdError>`。frb 把 `Err` 映射为 Dart 异常；wasm-bindgen 把 `Err` 映射为 JS 异常（`JsError`）。
  Dart 门面统一包成 `MarkdownCodecException`。

### 3.2 JSON 形状

```jsonc
// BlockJson —— 与 Rust Block 的 serde 形状相同，但不带 id / children（门面层不关心）
{ "type": "paragraph", "text": "hello", "data": { "marks": [ { "start": 0, "end": 5, "type": "bold" } ] } }
// data 永远是对象，空时 {}；marks 保持解析器产出顺序（§3.4），对象键按字典序

// InlineJson
{ "text": "hello", "marks": [ { "start": 0, "end": 5, "type": "link", "href": "https://x", "title": "t" } ] }

// CellJson —— 表格单元格存储形态（§6.4），原样传递：字符串或 { "text", "marks" }
```

这与 AI 编辑接口里服务端返回的 `replacement` 块形状一致（`ai_edits.rs` → Dart `AiBlockEdit.fromJson`）。

### 3.3 函数表（`wire.rs`，绑定层同名转发）

| 函数 | 入参 | 出参 | 对应 API |
| --- | --- | --- | --- |
| `parse_body_json(markdown: &str, fragment: bool) -> Result<String>` | Markdown | `[BlockJson]` | `parse_body` |
| `parse_inline_batch_json(sources_json: &str) -> Result<String>` | `["src", …]` | `[InlineJson]` | `parse_inline` ×N |
| `decode_cells_json(cells_json: &str) -> Result<String>` | `[CellJson]` | `[InlineJson]` | `decode_cell` ×N |
| `encode_cells_json(inlines_json: &str) -> Result<String>` | `[InlineJson]` | `[CellJson]` | `encode_cell` ×N（批量：一次表格写入只跨一次边界） |
| `serialize_body_json(blocks_json: &str, assets_json: &str) -> Result<String>` | `[BlockJson]`、`{file_id: path}` | Markdown | `serialize_body` |
| `serialize_inline_json(inline_json: &str) -> Result<String>` | `InlineJson` | Markdown | `serialize_inline` |
| `render_html_body_json(blocks_json: &str, image_srcs_json: &str, profile: &str) -> Result<String>` | `[BlockJson]`、`{file_id: src}`、`"export"` / `"clipboard"` | HTML | `render_html_body` |
| `math_run_spans_json(source: &str) -> String` | 文本 | `[[s,e], …]`，**UTF-16 偏移**（由 char 下标换算） | `math_run_spans` |
| `parse_inline_math_json(source: &str) -> String` | 文本 | `InlineJson` | `parse_inline_math` |
| `engine_info_json() -> String` | — | `{"crate_version","schema_version","source_fingerprint"}` | 构建信息 |

`parse_body_json` 输出的块 id 不出现在 JSON 里（Dart 插入时自行分配），所以内部用 `BlockIds::Sequential`，**不需要随机数**。

### 3.4 规范化规则

- `marks` **数组顺序有语义，线格式不重排**（G3 裁决 B-1，2026-10-28，原为「按 `(start, end, type)` 排序输出」）：
  同一区间有多个标记时，序列化器按数组下标决定谁在外层（解析器把外层标记排在后面）。重排会改变嵌套，
  例如 fixture 43 的「图片-链接-图片」重新导出后变成链接包图片。编辑器 `_normalize` 在改动标记时按 `(type, start)` 排序，
  对两个同区间标记的结果与原排序一致，只有三个及以上同区间、且类型重复时才有差别。
- **比较**标记时（黄金文件、差分、`marksEqual`、往返测试）一律先按 `(start, end, type)` 排序再比，不比顺序。
- `data` 中的键顺序无意义；比较时按值比较。
- 文本换行统一 `\n`（Dart 粘贴前已把 `\r\n` 归一，见 `editor.dart` 粘贴路径）。

## 4. 绑定

### 4.1 桌面：FFI（flutter_rust_bridge 2.12.0，现有管线）

新文件 `clients/mica_flutter/rust/src/api/markdown.rs`：

```rust
//! Markdown codec for the desktop editor — one-line forwards to `mica_md::wire`,
//! so desktop and web (crates/md-wasm) answer byte-identically.
use flutter_rust_bridge::frb;

#[frb(sync)] pub fn md_parse_body(markdown: String, fragment: bool) -> Result<String, String> {
    mica_md::wire::parse_body_json(&markdown, fragment).map_err(|e| e.to_string())
}
#[frb(sync)] pub fn md_parse_inline_batch(sources_json: String) -> Result<String, String> { … }
#[frb(sync)] pub fn md_decode_cells(cells_json: String) -> Result<String, String> { … }
#[frb(sync)] pub fn md_encode_cells(inlines_json: String) -> Result<String, String> { … }
#[frb(sync)] pub fn md_serialize_body(blocks_json: String, assets_json: String) -> Result<String, String> { … }
#[frb(sync)] pub fn md_serialize_inline(inline_json: String) -> Result<String, String> { … }
#[frb(sync)] pub fn md_render_html_body(blocks_json: String, image_srcs_json: String, profile: String) -> Result<String, String> { … }
#[frb(sync)] pub fn md_math_run_spans(source: String) -> String { mica_md::wire::math_run_spans_json(&source) }
#[frb(sync)] pub fn md_parse_inline_math(source: String) -> String { mica_md::wire::parse_inline_math_json(&source) }
#[frb(sync)] pub fn md_engine_info() -> String { mica_md::wire::engine_info_json() }
```

- `rust/src/api/mod.rs` 加 `pub mod markdown;`；`rust/Cargo.toml` 加 `mica-md = { path = "../../../crates/md-core" }`。
- 生成：在 `clients/mica_flutter` 下运行 `flutter_rust_bridge_codegen generate`（版本 2.12.0，与 pubspec 一致），
  提交生成的 `lib/src/rust/api/markdown.dart`、`frb_generated*.dart`、`rust/src/frb_generated.rs`。
- `rust/build.rs` 的源码指纹 ROOTS 加入 `crates/md-core/Cargo.toml`、`crates/md-core/src`（第 1 条的防旧库机制）。

### 4.2 Web：`crates/md-wasm`（wasm-bindgen）

```rust
// crates/md-wasm/src/lib.rs
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = parseBody)]
pub fn parse_body(markdown: &str, fragment: bool) -> Result<String, JsError> {
  mica_md::wire::parse_body_json(markdown, fragment).map_err(|e| JsError::new(&e.to_string()))
}
// … 其余函数同 §3.3，js_name 用 camelCase：parseInlineBatch / decodeCells / encodeCells /
//    serializeBody / serializeInline / renderHtmlBody / mathRunSpans / parseInlineMath / engineInfo
```

```toml
# crates/md-wasm/Cargo.toml
[package]
name = "mica-md-wasm"
version = "0.1.0"
edition.workspace = true
license.workspace = true
publish = false
[lib]
crate-type = ["cdylib", "rlib"]
[dependencies]
mica-md = { path = "../md-core", default-features = false }   # 无 uuid、无 render
wasm-bindgen = "=0.2.X"   # 精确钉版本；X 由 S2 选定（发布满两周以上的最新版），与 wasm-bindgen-cli 同号
# 体积优化在根 Cargo.toml 的 [profile.release.package.mica-md-wasm] 设 opt-level = "z"、codegen-units = 1；
# lto 不能按 package 设，由 build-md-wasm.sh 用 CARGO_PROFILE_RELEASE_LTO=true 只在构建 wasm 时打开（S2 T2.3）
```

- 加入根 workspace members（宿主机上也能编译，clippy 照常覆盖）。
- `rust-toolchain.toml` 加 `targets = ["wasm32-unknown-unknown"]`。
- 构建脚本 `scripts/build-md-wasm.sh`（S2 T2.6）：`cargo build -p mica-md-wasm --target wasm32-unknown-unknown --release --locked`
  → `wasm-bindgen --target no-modules --no-typescript --out-dir clients/mica_flutter/web/mica_md --out-name mica_md …wasm`
  → 校验 gzip 体积 ≤ 预算 → 写 `SOURCE_FINGERPRINT` → 复制一份到 `clients/mica_flutter/test/support/mica_md/`（浏览器测试用）。

### 4.3 为什么 Web 不用 flutter_rust_bridge 的 web 构建（D5）

1. 现有 FFI crate 依赖 SQLite（`libsqlite3-sys`），整体编不进 wasm；frb web 需要为它另起一个 frb 工程。
2. frb `build-web` 默认 `-Z build-std`（nightly）；免 nightly 的路径要绕开它的构建命令。我们的工具链钉死 stable（`rust-toolchain.toml`）。
3. 仓库已有成熟的 Web 接入模式：yjs 通过 `<script>` 暴露全局对象 `globalThis.micaYjs`，Dart 用 `dart:js_interop` 的 extension type 调用
   （`lib/web/yjs_interop.dart`）。wasm-bindgen `--target no-modules` 产出的全局 `wasm_bindgen` 对象可以用同一模式接入，
   依赖面最小（一个 Rust crate + 一个同号 CLI）。
4. 代价：Web 端多写一个约 80 行的 `markdown_codec_web.dart`（extension type 声明）。可接受。

### 4.4 Web 加载

`clients/mica_flutter/web/index.html`，放在 yjs 脚本之后、`flutter_bootstrap.js` 之前：

```html
<!-- Markdown engine (crates/md-core via crates/md-wasm). Starts compiling before
     Flutter boots; Dart awaits micaMdReady before the first frame. -->
<script src="mica_md/mica_md.js"></script>
<script>
  globalThis.micaMdReady = wasm_bindgen({ module_or_path: 'mica_md/mica_md_bg.wasm' })
    .then(() => { globalThis.micaMd = wasm_bindgen; })
    .catch((e) => { globalThis.micaMdError = String(e); });
</script>
```

（`wasm_bindgen(...)` 初始化参数的具体形式以选定版本的 wasm-bindgen 文档为准；S2 T2.7 落实并在执行记录里注明。）

## 5. Dart 门面（S2 交付并冻结）

```dart
// lib/markdown/markdown_codec.dart
import 'mark.dart';   // class Mark 自 editor/marks.dart 迁来（S2 T2.5）；marks.dart re-export，旧 import 方零改动
import 'markdown_codec_io.dart'
    if (dart.library.js_interop) 'markdown_codec_web.dart' as platform;

/// 块的门面形态。与原 `editor/markdown.dart` 的 `BlockSpec` 是同一个 typedef（迁移过来），调用点零改动。
typedef BlockSpec = ({String kind, String text, Map<String, dynamic> data});
typedef InlineText = ({String text, List<Mark> marks});

class MarkdownCodecException implements Exception {
  const MarkdownCodecException(this.message);
  final String message;
  @override String toString() => 'MarkdownCodecException: $message';
}

/// Markdown ⇄ 块 / 行内 / HTML。唯一实现在 Rust（crates/md-core）；桌面经 FFI，Web 经 wasm。
/// 全部同步：调用方都在用户操作的边界上（粘贴、复制、导出、AI），一次操作一次调用。
abstract interface class MarkdownCodec {
  List<BlockSpec> parseBlocks(String markdown, {bool fragment = false});
  InlineText parseInline(String source);
  List<InlineText> parseInlineBatch(List<String> sources);
  List<InlineText> decodeCells(List<Object?> cells);       // 表格单元格存储形态 → 文本+标记
  List<Object> encodeCells(List<InlineText> cells);         // → 每个 String 或 {text, marks}（批量）
  String serializeBlocks(List<BlockSpec> blocks, {Map<String, String> assets = const {}});
  String serializeInline(String text, List<Mark> marks);
  /// 剪贴板 text/html（`HtmlProfile::Clipboard`）。Dart 只有这一个 HTML 用途；导出 HTML 走现有 FFI `export_html_document`。
  String renderClipboardHtml(List<BlockSpec> blocks, {Map<String, String> imageSrcs = const {}});
  List<({int start, int end})> mathRunSpans(String source);   // UTF-16
  InlineText parseInlineMath(String source);
  ({String crateVersion, int schemaVersion, String sourceFingerprint}) get engineInfo;
}

MarkdownCodec? _codec;
MarkdownCodec get markdownCodec =>
    _codec ?? (throw StateError('initMarkdownCodec() was not awaited before first use'));

/// 桌面：确保 RustLib 已初始化（复用 initNativeRust，幂等）。Web：等待 globalThis.micaMdReady（10 s 超时）。
/// 失败不抛：记录 crash log，并安装 [UnavailableMarkdownCodec]（每个方法抛 MarkdownCodecException），
/// 让调用方按 §5.2 降级。启动不能因为它失败（lessons：启动关键路径）。
Future<void> initMarkdownCodec() async { /* 失败兜底与计时，完整实现见 S2 T2.5 */ }

@visibleForTesting
void debugSetMarkdownCodec(MarkdownCodec? codec) => _codec = codec;
```

- `markdown_wire.dart`：`List<BlockSpec> decodeBlocks(String json)`、`String encodeBlocks(List<BlockSpec>)`、
  `InlineText decodeInline(Map)`、`Map encodeInline(InlineText)` 等纯函数。两端实现共用它，只在「调哪个原生函数」上不同。
- 测试支撑：`test/support/markdown_codec.dart` 提供 `Future<void> useNativeMarkdownCodec()`（`initCheckedNativeRust()` + 安装 io 实现）。
  **不提供纯 Dart 假实现**——那等于第三份实现。需要门面的单元测试一律走真 FFI（门禁已经保证原生库是新鲜的）。

### 5.1 启动接线

`lib/main.dart` 的 `main()` 里，`runApp` 之前：`await initMarkdownCodec();`。
`main.dart` 库在行数棘轮上（只许减不许增）：**加这一行的同一个 PR 里**必须在同库别处净减一行（S2 T2.8），不能留给后面的阶段抵消。

### 5.2 降级（引擎不可用时）

| 场景 | 行为 |
| --- | --- |
| 粘贴 Markdown / HTML | 退回纯文本插入（现有第 3 步「Plain text」路径），SnackBar 提示一次 |
| 复制 | text/plain 照常（不依赖引擎）；text/html 与 Markdown flavor 省略 |
| AI 读取当前页 | 报错提示，不发送 |
| 表格旧格式单元格（字符串） | 原文显示（不解析），编辑时按纯文本处理 |

## 6. 块 / 标记 Schema（规范，S1 由 `schema.rs` + `schema_sync.rs` 落地）

### 6.1 块类型（`type`）

**根块的 type 不可靠**（G1 裁决 #2）：编辑器和服务端新建的页面根块是 `page`，但解析器（`parse` / 现有 `import_markdown`）
产出的根块一直是 `paragraph`，导入过的页面在库里就是这样存的。消费方只能用 `root_block_id` 识别根块，不能看 type。
统一根块 type 会改动已存数据的读法，不在本专项范围。

| type | 含义 | text | data 键（解析器产出 / 导出器读取） |
| --- | --- | --- | --- |
| `page` | 根块（编辑器 / 服务端新建的页面） | 通常为空（旧文档可能有正文） | `front_matter`（原文 YAML）、`title`（Mica 适配层） |
| `paragraph` | 段落 | 行内文本 | `marks`、`li`、`quote`、`qbreak` |
| `heading` | 标题 | 行内文本 | `level`（1–6）、`marks`、`quote`、`qbreak` |
| `bulleted_list` | 无序列表项 | 行内文本 | `indent`、`loose`、`marker`、`marks`、`quote`、`qbreak` |
| `numbered_list` | 有序列表项 | 行内文本 | `indent`、`loose`、`marker`、`start`、`marks` |
| `todo` | 任务项 | 行内文本 | `checked`、`indent`、`loose`、`marks` |
| `quote` | 引用 | 行内文本 | `quote`（深度）、`qbreak`、`alert`（GFM：note / tip / important / warning / caution）、`li`、`marks` |
| `code_block` | 代码块；**原样 HTML 块也是它**（`language: "html", raw: true`）；mermaid 是 `language: "mermaid"` | 原文 | `language`、`raw`、`li`、`quote`、`qbreak` |
| `math_block` | 显示公式 | LaTeX 原文 | — |
| `image` | 图片 | alt | `url`、`title`、`file_id`（Mica 资产）、`name` |
| `table` | GFM 表格 | — | `rows`（§6.4）、`header`、`align`、`aligns`、`widths`（编辑器写，`width` 同） |
| `divider` | 分隔线 | — | `li` |
| `footnote_def` | 脚注定义 | 定义正文 | `label`、`marks` |

**只读兼容的别名**（导出器接受、解析器不产出）：`bullet_list` = `bulleted_list`、`number_list` = `numbered_list`、`code` = `code_block`
（`export_md.rs:60/212/216/224`）。未知 kind / 未知 data 键：导出器忽略，不报错（编辑器可能写入 Dart 专属键，例如表格 `width`）。

> 上表核对过 `block.rs` / `block_handlers.rs` 的全部 `push_block` 调用、`classify_markdown_line` 的返回值、导出端的 kind 分支，
> 以及 105 个黄金文件的实际产出。注意 `block.rs:713` 起的 `HTML_BLOCK_TAGS`（`caption`、`details`、`summary`…）是 **HTML 标签名**，不是 data 键。
> **S1 T1.6** 把它落成 `schema.rs` 常量，并让 `schema_sync.rs` 校验：fixtures 里出现的任何 kind / data key / mark type 都在常量表里。

### 6.2 嵌套模型（扁平）

块表是扁平的：正文块全部是根块的 `children`。层级通过 data 表达：`indent`（列表层级）、`quote` / `qbreak`（引用深度 / 分组）、
`li`（列表项的容器子块）。HTML 导出时重建嵌套。深度上限 `MAX_NESTING_DEPTH = 8`。

### 6.3 标记类型（`marks[].type`）

`bold`、`italic`、`strike`、`code`、`link`（`href`、可选 `title`；允许零宽）、`image`（行内图片）、`math`、`html`（行内 HTML）、`footnote`（脚注引用）。
偏移为 UTF-16 code unit，`[start, end)`。

### 6.4 表格单元格存储形态

`data.rows: [[cell, …], …]`，每个 `cell` 是以下之一（**现状，S4 不改**）：

| 形态 | 含义 | 读 |
| --- | --- | --- |
| JSON 字符串 | 无格式单元格，或旧文档：**内容是行内 Markdown 源码** | `decode_cell` 解析它（`**x**` 会变成加粗） |
| `{"text": …, "marks": [...]}` | 带格式单元格（Rust `encode_table_cell`、Dart `_encodeCell` 都这样写） | 直接取 |
| `null` / 缺失 / 其他 | 空单元格 | 空 |

`encode_cell(x)`：`x.marks` 非空 → 对象；为空 → `serialize_inline(x.text, [])`（转义后的源码字符串，保证 I3）。
**已知不无损的两类**（I3 的除外项）：无标记的裸 URL / 邮箱读回会多一个 `link` 标记，首尾空白读回会被去掉。
两者都是现有存储行为，旧客户端也这样读；是否改（例如对这类单元格改存 `{text, marks: []}` 对象）在 S4 决定，见 S4 T4.8。
**注意**：无格式但含字面 `*` 的文本必须转义后再存成字符串，否则读者会把它解析成格式。

## 7. 性能预算（S0 测基线，S2/S3/S4 验收）

| 指标 | 预算 | 测量方式 |
| --- | --- | --- |
| 桌面 `initMarkdownCodec()` | ≤ 30 ms（p50，Windows 发布版） | 启动日志计时 |
| Web wasm 实例化 + `micaMdReady` | ≤ 60 ms；首帧推迟 ≤ 60 ms | Playwright `performance.now()` |
| wasm 体积 | ≤ 256 KiB（262,144 B）gzip，wasm 与 JS 分别 `gzip -n -9` 后相加；目标 ≤ 120 KB（M2 用户决定，原 200 KB） | 构建脚本强制；以 Linux（CI 平台）构建为准 |
| 粘贴 10k 字符 Markdown（解析） | ≤ S0 Dart 基线 × 1.2（桌面、Web 各自对比） | `test/perf/markdown_codec_bench_test.dart` + 浏览器版 |
| 复制 10k 字符选区（序列化 Markdown + HTML，两者合计） | 桌面、Web 各自 ≤ 16 ms（一帧）；与 Dart 的比值只记录、不作门槛（M2 用户决定，原 ≤ S0 基线 × 1.2） | 同上 |
| 500 单元格表格首次解码（旧字符串格式） | 桌面 ≤ 16 ms、Web ≤ 30 ms（批量接口）；之后每帧 0 次引擎调用 | S4 基准测试 |
| 单次边界调用开销（87 字符行内） | 桌面 ≤ 0.1 ms、Web ≤ 0.2 ms | 同上 |

任何一项超预算：先查是否是 JSON 往返的开销（可改为批量接口），再报审核方，**不要**为了达标在 Dart 侧加缓存版解析器（那是第三份实现）。
