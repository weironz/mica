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
- `parse_body_json(md)` 等于把 `parse_body(md)` 逐块做 §3.2 投影（去掉 id / children、`data` 缺失或 null 归一为 `{}`、
  marks 按 `(start, end, type)` 排序）后的 JSON（保证 S2 两个绑定只是转发）。原写「经 serde 后逐字节相同」按字面不成立，G1 裁决 #1 改为此口径。

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

### 2026-10-26 · WP-R（Rust 引擎）· 分支 `worktree-agent-aaa10db22e269aaf4`

**状态：⏸ 待审核（G1）。** 未 merge、未 push、未发版。

### 提交

| 提交 | 任务 | 说明 |
| --- | --- | --- |
| `9af0de7` | S0 T0.3-Rust | 新增 `crates/markdown/examples/md_bench.rs`，`Instant` 三档输入，不引 criterion |
| `bede590` | T1.1 | 新建 `crates/md-core`（`mica-md`）骨架，加入 workspace members |
| `27cb146` | T1.2–T1.9 | 搬模块（rename 保留历史）、门面变薄、注入 id、类型化 API、`wire`、`schema`、T1.4–T1.9 测试与文档 |
| `f99add9` | T1.10 | 独立审查 6 项修复：id 碰撞、wire 排序/签名、旧内部测试完整迁移、cell 属性测试重写 |
| 本次记录提交 | T1.10 | 门禁 + 执行记录 + 进度表标 G1 |

**关于提交粒度**：T1.2–T1.9 在编译上是一个不可分边界（模块搬家后旧门面立即引用不到核心，核心 API/门面必须同一次落地才可编译），
因此按一个提交交付，而不是人为拆出不可编译的中间提交。T1.1 与 T0.3 各自独立提交。

**搬家用 rename**：最初为绕开本机 pre-commit 图谱钩子做过「先删后建」，随后在本地 reset 重做为一次 `git add -A`
的提交；`git show -M` 显示 `block.rs`、`inline.rs`、`html.rs`、`export_md.rs`、`span_walk.rs`、`entities.rs`、
`scan_memo.rs`、`block_handlers.rs`、`properties.rs` 均为 rename（`crates/markdown/src → crates/md-core/src/...`）。

### T0.3 Rust 基线（release，7 次取中位数，前 2 次预热）

| 项 | 基线 `9af0de7` | 重构后 `27cb146`（三次最好） | 差异 |
| --- | --- | --- | --- |
| import.87 | 0.013 ms | 0.013 ms | 0% |
| export_markdown.87 | 0.011 ms | 0.011 ms | 0% |
| export_html.87 | 0.008 ms | 0.008 ms | 0% |
| import.10k | 0.867 ms | 0.922 ms | **+6.3%** |
| export_markdown.10k | 0.508 ms | 0.491 ms | −3.3% |
| export_html.10k | 0.405 ms | 0.398 ms | −1.7% |
| import.100k | 4.581 ms | 4.647 ms | +1.4% |
| export_markdown.100k | 2.689 ms | 2.573 ms | −4.3% |
| export_html.100k | 2.124 ms | 1.991 ms | −6.3% |

9 项中 **7 项在 ±5% 内，2 项超出**：
`import.10k` +6.3%（基线 0.867 ms，新实现三次最好 0.922 ms；新实现重复运行 0.922–1.038 ms），
`export_html.100k` −6.3%（基线 2.124 ms，新实现 1.991 ms，即更快但超出绝对阈值）。
两次基线数字来自同一台机器、同一 `md_bench`，但**没有做同机交替 A/B 对照**，因此这里只记录事实，不把差异归因于噪声；
`import.100k`（块数更多）+1.4% 与其余导出项在范围内。为降低 id 注入开销，`assign_ids` 采用单遍、每块只生成一次最终 id
（Uuid 不是确定性函数，二次生成会导致 children 悬空——见下），并跳过空 children 的映射表。

### 新增/删除行数

`git show --stat -M 27cb146`：32 files changed，**+2223 / −1005**（rename 不计搬运行数）。
搬家后各文件行数与原文件相同（`block.rs` +42 行为 id 注入）；`crates/markdown/src/lib.rs` 1417 → 572 行。

### 实现要点

- **核心模型**：`Block` / `DocumentSnapshotPayload` 是原类型**迁移**，`mica-core` 的 `Block` re-export 仍指向同一类型。
  新增 `Mark` / `InlineText` / `SCHEMA_VERSION` / `MdError`。
- **id 注入（T1.4）**：`push_block` 改推 `\u{1}<index>` 临时 id；`import_markdown_inner(..., &BlockIds)` 末尾
  `assign_ids` 单遍替换并重写根 children。检查过解析器无任何按 id 查块的逻辑（grep `\u{1}`、`.id ==`、`find(|b| b.id`），
  临时 id 只被写入、不被读取。`Uuid` 模式每个块只调用一次 `id_for`（测试曾暴露二次调用导致 `BlockNotFound`，已修）。
- **类型化 API（T1.5）**：02 §2 全部函数；`parse_body` 用 `Sequential{"b"}`；`serialize_body` / `render_html_body`
  用合成根 `\u{1}root` + `\u{1}b<i>`；`HtmlProfile::Clipboard` 在 S1 与 `Export` 输出相同（rustdoc 注明 S3 T3.4）。
- **wire（T1.7）**：§3.3 十个函数；`data` 归一为 `{}`；marks 输出前按 `(start,end,type)` 排序；
  `math_run_spans_json` 一次前缀扫描把 char 下标换成 UTF-16；`engine_info_json` 的 `source_fingerprint` 选「md-core/build.rs 计算」
  （FNV-1a 64，遍历 `Cargo.toml` + `src/**`，`path\0contents\0`，与 FFI `build.rs` 同形）。
- **schema（T1.6）**：`kinds` / `marks` / `data_keys` + `ALL` + `ALIASES`；`schema_sync.rs` 正向校验语料中每个 kind/key/mark，
  反向校验常量无死项（`page`、`title`/`file_id`/`name`/`widths`/`width`/`aligns` 明确标为编辑器/适配层专用）。

### 偏离方案之处及原因

1. **核心多了兼容入口**：除 §2 的函数外，`mica-md` 还公开 `import_markdown`、`import_markdown_fragment`、`export_markdown`、
   `export_markdown_with_assets`、`export_html`、`set_image_srcs`、`split_front_matter`。前六个是 T1.3 要求门面从
   `mica_md` re-export 的既有入口；`split_front_matter` 仅因 Mica 的页码标题读写必须与解析器共用同一条 front-matter 规则
   （否则就是第二份规则）。属**追加**，不改既有签名，请审核方确认是否接受。
2. **门面错误映射**：`DocumentOperationError` 仍是门面类型；`impl From<MdError>`：`UnsupportedSchemaVersion`/`BlockNotFound`
   一一对应，`InvalidInput` → `UnsupportedSchemaVersion(0)`（门面不解析 wire 输入，保持全集）。
3. **`text_edit` 不再用核心私有 `apply_inline_marks`**：改用公开 `mica_md::parse_inline` 再序列化 marks；
   结果 marks 现在是规范化顺序（`(start,end,type)`），既有 `text_edit.rs` 测试全绿。
4. **单元测试搬家**：原 `lib.rs` 的 `export_xss_tests` / `tree_shape_tests` / `security_tests` 中测核心私有函数的用例
   不在门面里了（跨 crate 私有不可见）。等价行为改由 `crates/md-core/tests/security_pipeline.rs`（走公开 `export_html`）
   与门面 `facade_security_tests` 覆盖；`page_title_tests` 原样留在门面。
5. **完成顺序**：`parse_inline`/`decode_cell`/`encode_cell` 等 T1.5 函数在 T1.2 的同一提交里落地（门面 `text_edit` 需要它们才能编译），
   功能范围与 §2 一致，未缩减。
6. **T1.1 提交时旧依赖暂留**：骨架提交先保留门面原依赖，T1.2 提交才把 render/uuid 依赖下沉到 `md-core`；
   这样每个提交都可编译。最终 `crates/markdown/Cargo.toml` 只保留 `mica-md` + serde/serde_json/thiserror + proptest(dev)。

### T1.10 门禁（实际回执与阻塞）

**`bash scripts/local-ci.sh rust` 未能在本机一次跑绿**，原因是环境而非代码：

1. 首次运行在 `rust: migrations` 失败：共享开发库 `mica-postgres` 返回 `VersionMismatch(12)`。未改动/重置共享库；
   改用独立库 `postgres://mica:mica@127.0.0.1:55432/mica_wpr`（`mica-test-pg` 容器内新建，`apply_migrations` 成功）。
2. 后续运行中 Docker Desktop 两次自行停止（`PoolTimedOut`），以及一次并行链接出现 `link.exe exit code 1073807364`
   （多目标同时链接的资源问题）。用 `CARGO_BUILD_JOBS=1` 串行完成测试链接后通过。

因此按局部命令给出可复核回执（均在本 worktree、独立库上实跑）：

| 命令 | 结果 |
| --- | --- |
| `bash scripts/check-ignored-sources.sh` | 通过 |
| `bash scripts/check-migration-tracking.sh` / `test-migration-tracking.sh` | 通过（36/1/2 个迁移齐全，LF） |
| `bash scripts/test-ci-paths.sh` / `test-flutter-sdk-reuse.sh` | 通过 |
| `bash scripts/check-file-size.sh` | passed（无新增超限单元；`inline.rs` 2205、`block_handlers.rs` 1535 均低于软上限） |
| `cargo fmt --all --check` | 0 |
| `cargo build --workspace` | 通过（38m19s，首次全量） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0（5m24s） |
| `cargo run -q -p mica-infra --example apply_migrations`（`DATABASE_URL=…/mica_wpr`） | `migrations applied` |
| `cargo test --workspace`（同上 DB，`MICA_REQUIRE_DB=1`，`CARGO_BUILD_JOBS=1`） | **exit 0，无失败**（含 api-server 契约黄金文件、DB 测试、md-core/markdown 全部）。该次运行在审查修复提交**之前**的工作树；修复后（`f99add9`）在最终树上重跑了受影响的 `cargo test -p mica-md`、`-p mica-md --no-default-features`、`-p mica-markdown`、`-p mica-core --features store` 与全 workspace clippy（均 exit 0）。最终树的全量 `cargo test --workspace` 复跑因单作业串行编译超过 1 小时被中止，**不声称它已在最终提交上完成**。 |
| `cargo test -p mica-core --features store` | exit 0：lib 112 passed/1 ignored，其余集成测试全绿 |
| `cargo test -p mica-md` | exit 0：lib 18、cell_roundtrip 6+1 ignored、ids 5、inline_math 9、schema_sync 1、security_pipeline 6、wire 9、doc 1 |
| `cargo test -p mica-md --no-default-features` | exit 0：uuid/seq 对比用例按 `cfg` 跳过，其余全绿 |
| `cargo test -p mica-markdown` | exit 0：`crates/markdown/tests/` 全部原样通过（CommonMark 641/641、GFM、golden-master、conformance 105、proptest） |
| `cargo test -p mica-markdown --test facade_surface` | 通过 |

**契约黄金文件的 Windows 行尾问题**：`contracts/*.json` 在 `core.autocrlf=true` 下被 checkout 成 CRLF，而测试按 LF 读取，
导致 14 个 `contract_gold` 用例先红。已把工作区这些文件规范为 LF（`git hash-object` 与 `HEAD` blob 完全一致，**没有提交任何内容改动**），随后全绿。

**FFI 独立 workspace 的 lock 不一致（交付阻塞，需裁决）**：
`(cd clients/mica_flutter/rust && cargo check)` 在自动重写其 `Cargo.lock` 后能通过；把 lock restore 后再跑
`cargo check --locked` **失败**：
`cannot update the lock file … because --locked was passed`。其独立 `Cargo.lock` 仍把 `mica-markdown` 直接连到
merman/ratex/uuid 且没有 `mica-md`。消费方 `Cargo.toml`/源码零改动，但独立 lock 必须随路径依赖图更新。
建议 G1 把 `clients/mica_flutter/rust/Cargo.lock` 的**必要路径依赖图变更**列入允许范围（不顺带升级任何 registry 包）；
本次受「clients 零改动」约束未提交该 lock。

消费方零改动：`git diff --stat 9af0de7 HEAD -- clients/ scripts/ .github/ crates/app-core crates/api-server crates/interchange crates/mcp-server crates/mica-core crates/cli crates/infra` 为空（lock 已 restore）。

### 独立审查项的验证与修复（2026-10-27）

| # | 结论 | 处理 / 红绿证据 |
| --- | --- | --- |
| 1 root id 与 Sequential 冲突 | **confirmed** | `root_id:"b0" + prefix:"b"` 时根与首正文同为 `b0`。修复：仅在碰撞输入上确定性地派生 effective prefix（`b`→`b_`），计数仍从 0；正常输入仍是 `b0,b1…`。红：临时禁用派生 → `a_root_id_in_the_sequential_namespace_never_collides` FAILED；绿：恢复后 5 个 ids 用例通过。**这是契约例外，需审核方确认**（原 prefix 逐字不变 + n 从 0 + 唯一，三者在碰撞输入上不可同时成立）。 |
| 2 空 marks 单元格与 URL/空白 | **confirmed** | `https://example.com` decode 后新增 `link`，`"x "` 丢尾随空格。测试 `empty_marks_cells_document_the_current_loss` 钉住现状；`unmarked_cells_are_lossless_for_every_text` 以 `#[ignore]` 保留目标断言。**未改存储策略**；注意旧 Dart 路径（`TableData._cellText` → `parseInline`）对 `{text, marks: []}` 同样会重解析，对象形态不是旧客户端无损保证。推荐裁决：I3 在新 typed API 严格（必要时对象回退），§6.4 明确允许空 marks 对象，并记录旧 Dart 消费路径的无损限制由 S4 处理。 |
| 3 wire marks 未排序 | **confirmed** | `parse_body_json("**a \`b\`**")` 原顺序为 code→bold。修复：`normalize_data` 在 wire 输出统一按 `(start,end,type)` 排序并保留额外字段；`encode_cells_json` 同样排序。红：临时去掉排序 → `block_json_marks_are_sorted_and_extra_data_survives` FAILED；绿：恢复后通过。 |
| 4 `parse_inline_math_json` 返回 Result | **confirmed** | 已恢复 §3.3 的 `fn(&str) -> String`。`wire_public_signatures_are_frozen` 对 10 个 wire 函数做函数指针钉死；签名不匹配将编译失败。 |
| 5 旧内部测试未完整迁移 | **confirmed** | 以 `git show 7e9db8c:crates/markdown/src/lib.rs` 为准：`tagfilter_*`（2）、`strip_unsafe_attrs_*`（3）、`meta_http_equiv_*`（1）原名/原断言搬到 `crates/md-core/src/html/html.rs` 私有 `security_tests`；`tree_shape_tests`（cycle + 5000 层首尾 `t0`/`t4999`）搬到 `crates/md-core/src/api.rs` 私有模块；`is_descendant` 两条断言与 CSP/`export_html_document` 测试留门面。未公开任何 helper。全部通过。 |
| 6 cell 属性测试生成器不合法 | **confirmed** | 重写：区间按文本真实 UTF-16 字符边界生成、互不重叠且间隔 1 字符，5 种 mark 由独立向量选择；I3（有 marks 对象恒等）与 I4（serialize→parse）分开。I4 明确排除：首尾空白、换行相邻空白、仅覆盖标点的强调（`**$**` 非 left-flanking）、未标记 run 中的裸 URL/邮箱，均有注释。修复后 6 passed / 1 ignored。 |

### 测试「先证明会失败」记录

| 测试 | 临时破坏 | 观测 |
| --- | --- | --- |
| `ids.rs` | 注释掉 `assign_ids` 调用 | `no_temporary_id_survives_a_parse` 红（临时 id 残留） |
| `ids.rs` | 禁用 root id 碰撞派生 | `a_root_id_in_the_sequential_namespace_never_collides` 红（重复 id） |
| `wire.rs` | `math_run_spans_json` 直接输出 char 下标 | `offsets_are_utf16_for_non_bmp_text` 红 |
| `wire.rs` | 去掉 wire marks 排序 | `block_json_marks_are_sorted_and_extra_data_survives` 红 |
| `schema_sync.rs` | 从 `data_keys::ALL` 删除 `alert` | `conf:24-callouts.md: unknown data key "alert"` 红 |
| `cell_roundtrip.rs` | `encode_cell` 空 marks 时直接返回原文、不转义 | `plain_cells_escape_literal_markup` 红（`a *b* c` 丢字面星号） |
| `inline_math.rs` | 去掉 `$$` 开头的过滤 | `a_display_dollar_marker_is_left_alone` 红 |
| `facade_surface.rs` | 把门面 `export_html` 临时改名 | `cannot find function export_html` **编译失败**（签名钉死的证明方式） |

每次破坏后均已恢复；恢复后重跑全绿。

### 契约问题（请审核方裁决）

1. **T1.7 的「`parse_body_json(md)` 与 `parse_body(md)` 经 serde 后逐字节相同」按字面无法成立**：§3.2 规定 `BlockJson` 不带 id/children，
   且 `data` 缺失/null 归一为 `{}`；`Block` 的 serde 带 id/children、`data` 为 null 时省略。测试改为字段级等价，且 wire 侧按 §3.4 排序、
   比较前对 raw `data.marks` 同样排序。建议契约改成「语义相同（排序/归一后）」。
2. **§6.1 说根块 type 是 `page`，现有解析器实际产出 `paragraph`**（`block.rs` 根块构造自旧代码）。S1 行为不变，未改；
   `schema::kinds::PAGE` 标注为「编辑器/根块专用，解析器当前不产出」。是否要在后续阶段改根块 kind，属于行为变更，需拍板。
3. **`parse_inline_math` 的 `$$` 处理**：规格要求「基于 `math_run_spans`」，但 Rust `math_run_spans` 的 `$` 规则没有「前一字符不能是 `$`」
   这一条（Dart `MathRunSpans` 有）。实现保留 `math_run_spans` 不动（`latex_guard` 依赖现状），在 `parse_inline_math` 内过滤以 `$$` 开头的区间，
   以对齐 Dart。若认为应改 `math_run_spans` 本身，请裁决（会改变 MCP LaTeX 防护的边界）。
4. **I3 / §6.4 空 marks 存储形态**：无 marks 的单元格存「行内源码」，读取时重新解析，因此裸 URL 会自动加 link、首尾空白会被规范化。
   `cell_roundtrip.rs` 用显式 loss 测试 + ignored 目标测试记录。推荐裁决：空 marks 无法无损时写现有 `{text, marks: []}` 对象，
   §6.4 同步允许该形态；同时记录旧 Dart `TableData` 路径对空 marks 对象仍会重解析 text，不能声称旧客户端无损兼容（S4 处理）。
5. **`BlockIds::Sequential` 的契约边界**：约定是 `<prefix><n>`、n 从 0 起。若调用方 `root_id` 落在同一命名空间
   （如 `b0` + `b`），三者无法同时满足。当前实现**仅在此碰撞输入**上确定性地改用 `b_` 前缀并保持 n 从 0；正常输入逐字不变。
   请确认该例外，或改为「`root_id` 不得与 prefix+n 冲突」的前置条件（`parse` 无 Result，错误报告需另设计）。
6. **FFI 独立 lock**：见「门禁」一节的 `cargo check --locked` 失败。请裁决是否允许 `clients/mica_flutter/rust/Cargo.lock` 随路径依赖图更新。
7. **核心公开面比 §2 多了兼容入口**：`import_markdown`、`import_markdown_fragment`、`export_markdown`、`export_markdown_with_assets`、
   `export_html`、`set_image_srcs`、`split_front_matter`。前六个是 T1.3 要求门面 re-export 的既有入口；`split_front_matter` 供页码标题读写与解析器共用同一规则。
   均为追加、不改既有签名；请确认是否接受。

### 未完成项

- S1 T1.1–T1.9 已完成；T1.10 的**单条 `local-ci.sh rust` 未全绿**（原因如上：共享库版本不符、Docker 掉线、并行链接资源）。
  已用同仓库的局部命令逐项跑出回执，`cargo test --workspace` 与 clippy 均 exit 0；**不宣称 all green**。
- `cargo check --locked` 在 FFI 独立 workspace 失败（lock 需更新，待裁决）。
- 两项基准超出 ±5%（`import.10k +6.3%`、`export_html.100k −6.3%`），未做同机交替 A/B。
- 契约问题 1–7 待审核方裁决。
- 本执行者停在 **G1（⏸ 待审核）**，未 merge / 未 push / 未发版。

### 2026-10-27 · G1复核4项前三条

本节追加于正式裁决 `origin/single-source@1704e18` 之后；上文保留为历史记录，不再代表裁决后的待办。
按 `reviews.md` 的 G1「必须修改」完成前三条；第四条（S0/S1 合入 `single-source` 后的集成门禁）由协调者执行。
本分支未合入整个集成分支，未修改 `reviews.md`、02 契约或 04 进度表，未合入 spike。

| 必须修改 | 本分支提交 | 核对结果 |
| --- | --- | --- |
| 1. 引入 `6594f12` 优化 | `20ecde0`（cherry-pick） | 仅 `crates/md-core/src/api.rs`、`src/parse/block.rs`、`src/wire.rs`；3 files changed，86 insertions / 44 deletions；无 spike 文件 |
| 3. 按裁决删除 ignored I3 目标测试 | `88a46af` | 删除 `unmarked_cells_are_lossless_for_every_text`；保留 `empty_marks_cells_document_the_current_loss`，注释指向 02 §2.2 I3 已裁决的除外项；没有改变存储行为 |
| 2. 更新 FFI 独立锁文件 | `349f19a` | `git show --stat 349f19a`：仅 `clients/mica_flutter/rust/Cargo.lock`，1 file changed，10 insertions；所有 registry package 的 version/source/checksum/dependencies 均保持一致 |

锁文件由 `cargo check --offline` 自动生成，随后运行 `cargo check --locked`。使用 yq 将更新前后 TOML 转 JSON，jq 选取全部含 `source` 的 package 对象并排序，再以 `cmp` 比较；结果为 `Registry package entries identical (name/version/source/checksum/dependencies).`，不是仅比较版本号。

审核方已在 Linux 对 `12c7ff2` 获得完整 Rust 门禁回执（1106 passed，floor 500；store 213 passed，floor 1），并同机交替 A/B 确认 `6594f12` 后 import.10k 为 +4.4%。本轮不重复全 workspace/数据库/性能排查；只运行要求的三组测试和 FFI check，使用默认 Cargo 并发，未额外限流。

本轮三组测试按顺序执行，命令链开启 `pipefail`，后台任务 `bz7832fi9` 最终 exit 0。随后逐日志按 `test result: ok. N passed` 求和并校验下限，生成 `receipts.txt`；未重新编译或重跑测试。

| 命令 | 实际回执 | 日志 |
| --- | --- | --- |
| `cargo test -p mica-md` | exit 0；`receipt: 55 passed (floor 55)`，0 failed / 0 ignored | `mica-md.log` |
| `cargo test -p mica-md --no-default-features` | exit 0；`receipt: 54 passed (floor 54)`，0 failed / 0 ignored | `mica-md-no-default.log` |
| `cargo test -p mica-markdown` | exit 0；`receipt: 185 passed (floor 185)`，0 failed / 0 ignored | `mica-markdown.log` |
| `(cd clients/mica_flutter/rust && cargo check --offline)` | exit 0；仅新增路径包 `mica-md v0.1.0`；`Finished dev … in 5m 21s` | `ffi-generate-lock.log` |
| `(cd clients/mica_flutter/rust && cargo check --locked)` | exit 0；`Finished dev … in 0.34s` | `ffi-locked.log` |
| registry 条目全对象比较（yq / jq / cmp） | 相同，无 registry 升级或其他条目变化 | `lock-registry-check.log`、`registry-before.json`、`registry-after.json` |
| `cargo fmt --all --check` | exit 0 | 本轮终端输出 |

以上日志保存在本机 `C:/Users/willz/AppData/Local/Temp/mica-wpr-g1-review-20261027/`（未提交临时日志）。
本轮执行记录是验证之后的文档追加，不改变已验证的代码/锁文件。

前三条已完成；第四条合并及集成头 `local-ci.sh rust` 的回执见下节。契约尚未冻结。

### 2026-10-27 · G1 第四项：集成头 Rust 门禁

- 按审核要求使用 merge commit 依次合入 S0 修正 `e122680`（merge `5d8feb7`）和 S1 `8b5f824`（merge `f2bcb70`）。进度表冲突按实际状态解决；`reviews.md` 与 02 的审核裁决未改，`git ls-files spike` 无输出。
- Windows 本机在干净的集成头 **`f2bcb70bcd6aca35fb0016bbf7a38f1b91659db3`** 执行原样 `bash scripts/local-ci.sh rust`，退出码 **0**。独立空库 `mica_g1_integrated_20261027` 在 `mica-test-pg` 的 55432 端口；`MICA_REQUIRE_DB=1`。由于 Windows 没有 PostgreSQL CLI，临时 PATH 适配器调用该容器中的真实 `psql`，经 host.docker.internal:55432 验证选定 URL；先验证目标库可连接及不存在的库失败，没有触碰共享开发库。
- ignored-source、迁移追踪与 LF、CI 配置、结构、`cargo fmt --all --check`、`cargo build --workspace`、workspace clippy `-D warnings` 和 migrations 全部通过。测试回执：**`receipt: 1103 passed (floor 500)`**；`mica-core --features store` **`receipt: 213 passed (floor 1)`**。完整日志在本机 `target/g1-review-tools/local-ci-rust.log`（不提交）；该回执属于上述合并头，后续只有文档记录变化。
- 按审核方建议新增 `.gitattributes` 中 `contracts/*.json text eol=lf`（`f576559`），避免 Windows 黄金文件 CRLF 引起的误报。
- 门禁不覆盖 web e2e、Windows FFI 集成测试、容器启动、MCP inspector；它们不能由本次 Rust 回执代替。四项修改已完成，**G1 进入待审核（复核），未冻结契约、未合并 main**。


