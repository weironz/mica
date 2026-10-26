# S1 Rust 引擎按 SDK 形状拆分（5–7 天，行为不变）

## 目标

把 `crates/markdown` 拆成 **`crates/md-core`（package `mica-md`，SDK 核心）** + **`crates/markdown`（`mica-markdown`，Mica 适配 + 兼容门面）**，
并在核心上补齐 02 §2 的类型化 API 和 §3 的线格式模块。

**硬约束：行为不变、消费方零改动。** `app-core`、`api-server`、`interchange`、`mcp-server`、`mica-core`、FFI crate 一行都不改；
`crates/markdown/tests/` 下全部现有测试一行都不改且全部通过。这是本阶段「重构没改行为」的证明。

## 前置

- S0 的 Rust 基准（T0.3 的 `md_bench`）已有数字。
- 读 02 §1–§3、§6，03 §1。

## 任务

### T1.1 新建 crate 骨架

```
crates/md-core/
  Cargo.toml
  src/lib.rs
```

```toml
# crates/md-core/Cargo.toml
[package]
name = "mica-md"
version = "0.1.0"
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Markdown ⇄ block-document engine: CommonMark 0.31.2 + GFM + dialect, round-trip stable."
publish = false            # S5 由用户批准后才改

[features]
default = ["uuid"]
uuid = ["dep:uuid"]
render = ["dep:merman", "dep:ratex-parser", "dep:ratex-layout", "dep:ratex-svg", "dep:ratex-types"]

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
uuid = { workspace = true, optional = true }
# 下面五个原样搬自 crates/markdown/Cargo.toml，版本与特性不变
merman = { version = "0.7", features = ["render"], optional = true }
ratex-parser = { version = "0.1", optional = true }
ratex-layout = { version = "0.1", optional = true }
ratex-types = { version = "0.1", optional = true }
ratex-svg = { version = "0.1", features = ["embed-fonts"], optional = true }

[dev-dependencies]
proptest = "1"
```

- 根 `Cargo.toml` 的 `members` 加 `"crates/md-core"`。
- `crates/markdown/Cargo.toml`：依赖改为 `mica-md = { path = "../md-core" }`，原来的五个 render 依赖删掉，
  `render = ["mica-md/render"]`，`uuid` 依赖删掉（门面不再直接用）。`proptest` 保留在 dev-dependencies（现有 `proptest_parse.rs` 用）。
- 提交：`chore(md-core): T1.1 crate skeleton`。此时 `md-core/src/lib.rs` 只有 crate 文档注释，`cargo build --workspace` 通过。

### T1.2 搬模块（`git mv`，保留历史）

```
git mv crates/markdown/src/block.rs           crates/md-core/src/parse/block.rs
git mv crates/markdown/src/block_handlers.rs  crates/md-core/src/parse/block_handlers.rs
git mv crates/markdown/src/inline.rs          crates/md-core/src/parse/inline.rs
git mv crates/markdown/src/scan_memo.rs       crates/md-core/src/parse/scan_memo.rs
git mv crates/markdown/src/entities.rs        crates/md-core/src/parse/entities.rs
git mv crates/markdown/src/export_md.rs       crates/md-core/src/serialize/export_md.rs
git mv crates/markdown/src/span_walk.rs       crates/md-core/src/serialize/span_walk.rs
git mv crates/markdown/src/html.rs            crates/md-core/src/html/html.rs
git mv crates/markdown/src/properties.rs      crates/md-core/src/properties.rs
```

- 新建 `parse/mod.rs`、`serialize/mod.rs`、`html/mod.rs`，内容就是 `pub(crate) mod …;` 与 `pub(crate) use …::*;`，
  让被搬文件里的 `use crate::inline::*` 之类路径**改动最小**（改成 `use crate::parse::inline::*` 或通过 mod.rs 再导出均可，选改动行数少的）。
- `lib.rs` 里属于核心的部分搬到 `md-core`：
  - 模型：`DocumentSnapshotPayload`、`Block`（含 `new/with_*`）→ `md-core/src/model.rs`
  - `import_markdown`、`import_markdown_fragment`、`export_markdown`、`export_markdown_with_assets`、`export_html`、`set_image_srcs`、
    `export_safe_tree`（及其他被它们调用的私有函数）→ `md-core/src/api.rs`
  - `render_mermaid_svg*`、`MermaidHostTheme`、数学 SVG 渲染（`#[cfg(feature = "render")]` 那几段）→ `md-core/src/render.rs`
  - `lib.rs` 四个 `#[cfg(test)] mod`（`export_xss_tests`、`tree_shape_tests`、`page_title_tests`、`security_tests`）
    跟着它们测的函数走：测核心函数的去 `md-core`，测页面标题的留门面。
- `md-core` 内部函数可见性：被门面用到的核心内部函数，要么升级为 02 §2 里的公开 API（如 `decode_table_cell` → `decode_cell`），
  要么移到门面。**禁止**为了编译通过而开一个 `pub mod internal`。若确实绕不开，列入执行记录「契约问题」由审核方决定。
- 错误类型：核心函数返回 `Result<_, MdError>`；门面保持返回 `DocumentOperationResult`，用 `impl From<MdError> for DocumentOperationError`
  （`UnsupportedSchemaVersion` / `BlockNotFound` 一一对应；`InvalidInput` 门面不会遇到，映射为 `UnsupportedSchemaVersion(0)` 并加注释）。

### T1.3 门面 `crates/markdown/src/lib.rs`

```rust
//! Mica's Markdown adapter over the `mica-md` engine (crates/md-core): page
//! titles, the standalone HTML export, MCP text editing and the LaTeX guard,
//! plus a compatibility re-export of every engine item this crate used to
//! define — so app-core / api-server / interchange / mcp-server / mica-core /
//! the FFI crate compile unchanged. New code should depend on `mica-md`.

pub use mica_md::{
  Block, DocumentSnapshotPayload,
  export_html, export_markdown, export_markdown_with_assets, import_markdown, import_markdown_fragment,
  math_run_spans, properties, set_image_srcs,
};
pub use mica_md::render::{MermaidHostTheme, render_mermaid_svg, render_mermaid_svg_with_id};
// 无条件再导出：render 模块在 md-core 里无条件存在，函数体按特性切换（与现在 lib.rs:536/619 的写法一致）

mod latex_guard;
mod text_edit;
pub use latex_guard::{JSON_ESCAPE_COLLISIONS, mangled_latex_collision, mangled_latex_message};
pub use text_edit::{block_text_matches, block_text_preview, markdown_find_text, replace_block_text};

// 留在门面（Mica 专属）：DocumentOperationError、DocumentOperationResult、payload_from_value、
// document_title、set_document_title、with_page_title、strip_page_title、export_html_document、
// SAFE_HTML_CSP、block_index、is_descendant
```

**公开面不变的证明**：新增 `crates/markdown/tests/facade_surface.rs`，对 03 §1 列出的每一个 pub 项做「签名钉死」：

```rust
//! The facade must keep every item it exported before the md-core split, with
//! the same signature: app-core re-exports this crate wholesale
//! (`pub use mica_markdown::*`), so a dropped or changed item breaks callers
//! we cannot see from here. One line per item; a mismatch fails to COMPILE.
use mica_markdown::*;
#[test]
fn every_pre_split_item_keeps_its_signature() {
  let _: fn(&str, &str) -> DocumentSnapshotPayload = import_markdown;
  let _: fn(&str, &str) -> DocumentSnapshotPayload = import_markdown_fragment;
  let _: fn(&DocumentSnapshotPayload) -> DocumentOperationResult<String> = export_markdown;
  let _: fn(&DocumentSnapshotPayload, &std::collections::BTreeMap<String, String>) -> DocumentOperationResult<String> =
    export_markdown_with_assets;
  let _: fn(&DocumentSnapshotPayload) -> DocumentOperationResult<String> = export_html;
  let _: fn(&DocumentSnapshotPayload, &str, u32) -> DocumentOperationResult<String> = export_html_document;
  let _: fn(&mut DocumentSnapshotPayload, &std::collections::BTreeMap<String, String>) = set_image_srcs;
  let _: fn(&DocumentSnapshotPayload) -> Option<&str> = document_title;
  // … 03 §1 的全部函数逐个写；常量写 `let _: &str = SAFE_HTML_CSP;`；类型写 `let _ = Block::new("a", "b");`
  // properties 模块的五个项、text_edit 的四个、latex_guard 的三个也要写。
}
```

### T1.4 可注入的块 id（去掉核心对随机数的硬依赖）

现状：`parse/block.rs` 的 `push_block` 里 `format!("block_{}", Uuid::new_v4().simple())`，31 处调用。

做法（**不改 31 个调用点**）：

1. `push_block` 改用确定性的临时 id：`let id = format!("\u{1}{}", blocks.len());`（`\u{1}` 不会出现在任何真实 id 里）。
2. `import_markdown_inner(markdown, root_block_id, parse_front_matter)` 增加参数 `ids: &BlockIds`；末尾调用新函数
   `assign_ids(&mut payload, ids)`：遍历 `payload.blocks`，按顺序把临时 id 换成正式 id，同时改写所有 `children` 数组里的引用
   （根块的 children，以及任何块的 children——现在只有根块有，但按通用写法处理）。
   - `BlockIds::Uuid` → `block_<uuid-v4-simple>`（现有格式）
   - `BlockIds::Sequential { prefix }` → `<prefix><n>`，n 从 0 起
3. 解析过程中如有代码按 id 查块（grep `.id ==`、`find(|b| b.id`），确认它们在临时 id 下仍成立（临时 id 也唯一），在执行记录里列出检查结果。
4. 门面 `import_markdown` / `import_markdown_fragment` 传 `BlockIds::Uuid`，行为与现在一致。
5. 测试 `md-core/tests/ids.rs`（不变量 I5）：
   - `Sequential` 模式同输入两次结果完全相等；
   - 对全部 conformance fixtures：`Uuid` 与 `Sequential` 的结果在「把 id 统一抹成占位符」后完全相等；
   - 任何结果里不残留 `\u{1}`。

### T1.5 类型化 API（`md-core/src/api.rs`、`model.rs`）

逐个实现 02 §2 的函数。实现要点：

| 函数 | 实现 |
| --- | --- |
| `parse(md, opts)` | `import_markdown_inner(md, &opts.root_id, opts.mode == Document, &opts.ids)` |
| `parse_body(md, mode)` | `parse(md, Sequential{"b"}, root_id "root")` 后按根块 children 顺序取块，清空 `children` |
| `parse_inline(src)` | 调现有 `parse_inline_with(src, &RefDefs::new())`，把内部 `InlineMark` 转成公开 `Mark`，按 (start, end, kind) 排序 |
| `decode_cell(v)` | 现有 `decode_table_cell`，结果 `(text, data)` 转 `InlineText` |
| `encode_cell(x)` | 02 §6.4：`marks` 非空 → `{"text", "marks"}`；为空 → `Value::String(serialize_inline(&x.text, &[]))` |
| `serialize(doc, opts)` | 现有 `export_markdown_with_assets(doc, &opts.assets)` |
| `serialize_body(blocks, opts)` | 造合成文档：根块 id `"\u{1}root"`、kind `page`、children = 每个块的新 id（`"\u{1}b<i>"`，忽略入参 id 以免冲突），调 `serialize` |
| `serialize_inline(text, marks)` | 现有 `render_inline(&Block{kind: "paragraph", text, data: {"marks": …}})`（定义在 `inline.rs:2058`，`export_md.rs` 的 `table_cell_markdown` 也调它） |
| `render_html(doc, opts)` | clone → `set_image_srcs(clone, &opts.image_srcs)` → 现有 `export_html` |
| `render_html_body(blocks, opts)` | 同 `serialize_body` 的合成根 → `render_html` |
| `HtmlProfile` | S1 只定义枚举与参数透传：`Clipboard` 暂时与 `Export` 输出相同，rustdoc 注明「S3 T3.4 实现」。**不要**在 S1 移植剪贴板方言 |
| `parse_inline_math(src)` | 用 `math_run_spans`（char 下标）：输出 = 去掉每段定界符（`$`…`$` 去 1+1，`\(`…`\)` 去 2+2）的文本；每段一个 `math` 标记，偏移换算为 UTF-16。语义以 Dart `parseInlineMath`（`marks.dart`）为准对齐，见下方测试 |

`parse_inline_math` 测试：把 Dart `test/paste_formula_test.dart` 里与 `parseInlineMath` 直接相关的输入输出逐条移植成
`md-core/tests/inline_math.rs`（中文、emoji 前后缀各至少一例，验证 UTF-16）。**这些 Dart 用例 S3 才删除**，S1 只复制。

### T1.6 Schema 常量与同步测试

`md-core/src/schema.rs`：

```rust
//! The block-document schema (docs/single-source/02-architecture.md §6), as
//! constants. Normative: a kind / data key / mark type the engine emits but
//! this file does not list is a bug, and tests/schema_sync.rs fails on it.
pub mod kinds {
  pub const PAGE: &str = "page";
  pub const PARAGRAPH: &str = "paragraph";
  pub const HEADING: &str = "heading";
  pub const BULLETED_LIST: &str = "bulleted_list";
  pub const NUMBERED_LIST: &str = "numbered_list";
  pub const TODO: &str = "todo";
  pub const QUOTE: &str = "quote";
  pub const CODE_BLOCK: &str = "code_block";
  pub const MATH_BLOCK: &str = "math_block";
  pub const IMAGE: &str = "image";
  pub const TABLE: &str = "table";
  pub const DIVIDER: &str = "divider";
  pub const FOOTNOTE_DEF: &str = "footnote_def";
  pub const ALL: &[&str] = &[PAGE, PARAGRAPH, HEADING, BULLETED_LIST, NUMBERED_LIST, TODO, QUOTE,
                             CODE_BLOCK, MATH_BLOCK, IMAGE, TABLE, DIVIDER, FOOTNOTE_DEF];
  /// Read-only aliases the exporters accept (never emitted by the parser).
  pub const ALIASES: &[(&str, &str)] = &[("bullet_list", BULLETED_LIST), ("number_list", NUMBERED_LIST), ("code", CODE_BLOCK)];
}
pub mod marks { /* BOLD ITALIC STRIKE CODE LINK IMAGE MATH HTML FOOTNOTE + ALL */ }
pub mod data_keys { /* 02 §6.1 表里出现的全部键 + ALL */ }
```

`md-core/tests/schema_sync.rs`：对 02 §6 提到的全部语料（conformance fixtures、spec、gfm、golden-master）做 `parse`，
收集出现过的 kind / data key / mark type，断言都在 `ALL` 里。再反向检查：`ALL` 里的每一项至少在一个语料里出现过，
或者在常量旁边用注释说明「仅编辑器写入」/「仅别名」——防止常量表变成死清单。

同时把 02 §6.1 的表格按最终常量表核对一遍；有出入以代码为准修正文档（提交里说明）。

### T1.7 线格式模块 `md-core/src/wire.rs`

实现 02 §3.3 全部函数。要点：

- 输入 JSON 解析失败 → `Err(MdError::InvalidInput(<serde 错误信息>))`。
- `BlockJson` 的 `data` 缺失或为 `null` → 当 `{}`；输出时 `data` 永远是对象。
- `marks` 输出前排序（02 §3.4）。
- `math_run_spans_json`：char 下标 → UTF-16 偏移（一次线性扫描建立 char→utf16 前缀表）。
- `engine_info_json`：`crate_version` 用 `env!("CARGO_PKG_VERSION")`；`source_fingerprint` 在 S2 由绑定 crate 注入，核心里返回 `""`
  （或由 `md-core/build.rs` 计算核心源码哈希——二选一，选了哪个写进执行记录；推荐后者，算法与第 1 条 FFI 指纹相同）。

测试 `md-core/tests/wire.rs`：
- 每个函数至少一个正常用例 + 一个错误输入用例；
- 不变量 I8：含 `😀`、`𠀀` 的输入，标记偏移和 `math_run_spans_json` 的偏移等于 UTF-16 下标（测试里手算期望值）；
- `parse_body_json(md)` 与 `parse_body(md)` 经 serde 后逐字节相同（保证 S2 两个绑定只是转发）。

### T1.8 不变量测试

`md-core/tests/cell_roundtrip.rs`（I3、I4），用 proptest：

```rust
proptest! {
  #![proptest_config(ProptestConfig::with_cases(512))]
  #[test]
  fn cell_encode_decode_is_identity(text in "[a-z *_`~|\\\\\\[\\]()$\n中😀]{0,24}") {
    // marks: 由 text 的长度随机生成 0–3 个不重叠区间，kind 从 {bold, italic, code, strike, link} 选
    // 断言：decode_cell(&encode_cell(&x)) == x
  }
}
```

注意：`code` 标记与其他标记的嵌套、零宽 `link` 等边界，如果引擎本来就不能往返（例如被规范化合并），
在测试里**明确排除并写注释说明为什么**，不要静默放宽断言。排除项列入执行记录。

### T1.9 文档注释

- `md-core/src/lib.rs` 加 `#![warn(missing_docs)]`，所有公开项写 rustdoc（一两句话 + 不变量）。
- crate 文档首页：一段话介绍 + 一个最小示例（作为 doctest 编译运行）：

```rust
//! ```
//! use mica_md::{parse_body, serialize_body, ParseMode, SerializeOptions};
//! let blocks = parse_body("# Title\n\nSome **bold** text.", ParseMode::Document);
//! assert_eq!(blocks[0].kind, "heading");
//! let md = serialize_body(&blocks, &SerializeOptions::default());
//! assert_eq!(md, "# Title\n\nSome **bold** text.");
//! ```
```

（示例输出以实际引擎为准，先跑再写。）

### T1.10 门禁与收尾

- `bash scripts/local-ci.sh rust` 全绿（含 fmt、clippy `-D warnings`、DB 测试回执）。
- `(cd clients/mica_flutter/rust && cargo check)`：FFI crate 编译通过且**零改动**。
- `bash scripts/check-file-size.sh`：搬家后单文件行数不变；`lib.rs` 拆分后各文件 ≤ 软上限。
- `cargo run -p mica-markdown --release --example md_bench` 与 S0 基线对比，差异在 ±5% 以内（重构不应改变性能）。
- **停点 G1**（[04 §4.2](../04-parallel-plan.md)）：执行记录写好后，进度表标「⏸ 待审核」，WP-R 停下，等用户转告审核方。
  审核方通过后，由审核方在 `02-architecture.md` 顶部把「S1 结束时冻结」改为「**已冻结（日期、提交号）**」；执行记录里要写明
  最终公开 API 与 02 §2 的差异（应为零；有差异须经审核方同意）。G1 通过后 S0 + S1 并回 main。

## 验收

- [ ] `crates/markdown/tests/` 下所有文件 `git diff` 为空，且全部通过。
- [ ] `facade_surface.rs` 覆盖 03 §1 的每个 pub 项。
- [ ] `app-core` / `api-server` / `interchange` / `mcp-server` / `mica-core` / FFI crate 的 `git diff` 为空。
- [ ] `md-core` 不依赖 `uuid`（`--no-default-features`）也能编译并通过 `ids.rs`、`wire.rs`、`schema_sync.rs`：
      `cargo test -p mica-md --no-default-features`。
- [ ] 新增测试 `ids.rs`、`wire.rs`、`schema_sync.rs`、`cell_roundtrip.rs`、`inline_math.rs`、`facade_surface.rs` 全部通过，
      并且每个都**先证明能失败**（例如临时破坏一处实现，测试变红），在执行记录里写怎么证明的。
- [ ] CommonMark 记分牌、GFM、golden-master 数字不变。
- [ ] 契约已冻结。

## 风险与回退

| 风险 | 应对 |
| --- | --- |
| `pub use mica_markdown::*` 让某个之前没注意到的 pub 项成了别人的依赖 | `facade_surface.rs` + 全 workspace 编译兜住；FFI crate 单独 `cargo check` |
| 搬模块时 `pub(crate)` 跨 crate 不可见导致大量可见性调整 | 优先把门面需要的函数移到门面；必须公开的按 02 §2 设计成正式 API |
| 临时 id 方案遗漏某个按 id 查块的逻辑 | T1.4 第 3 步逐个检查；`ids.rs` 对全部 fixtures 做等价比较 |
| render feature 的条件编译在门面和核心之间错位 | `cargo build -p mica-markdown --features render` 与不带 render 各编一次（api-server 和 FFI 都开 render） |

回退：本阶段是纯重构，任一验收项不过就不合并；已合并后发现问题，`git revert` 整个阶段的合并提交即可（消费方零改动，回退无连带）。

## 执行记录

（执行方追加。）
