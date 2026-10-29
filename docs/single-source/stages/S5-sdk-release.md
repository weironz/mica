# S5 SDK 化交付（5–7 天；发布本身需用户批准）

## 目标

让 `mica-md` 达到「**随时可以发布**」：公开 API 文档齐全、Schema 规范成文、示例可运行、API 稳定性有测试守着、
npm 包能打出来、发布流程是一条命令。**真正的 `cargo publish` / `npm publish` 不在本阶段执行**（D3），由用户拍板后按 T5.12 操作。

## 前置

- S1 完成（API 已冻结）。S5 的大部分任务可以与 S2–S4 并行（见 04）；T5.7（npm 包）依赖 S2 的 `crates/md-wasm`。

## 待用户决定（S5 开始时先问，答案写进执行记录）

| # | 问题 | 推荐 |
| --- | --- | --- |
| Q1 | crate 名：`mica-md`（现用）还是别的？发布前要查 crates.io 是否被占用 | `mica-md`，被占则 `mica-markdown-engine` |
| Q2 | npm 包名 / scope | `@mica/md`（需要注册 scope）或 `mica-md` |
| Q3 | 是否同时发 Dart / Flutter 包 | **不发**（见 T5.11） |
| Q4 | 许可证 | 沿用 workspace 的 `MIT OR Apache-2.0` |
| Q5 | 源码托管：留在本 monorepo 的子目录发布，还是拆独立仓库 | 留在 monorepo（`cargo publish -p` 支持），`repository` 字段指向子目录 |
| Q6 | S1 为兼容 Mica 现有调用方追加的入口（`import_markdown`、`import_markdown_fragment`、`export_markdown`、`export_markdown_with_assets`、`export_html`、`set_image_srcs`、`split_front_matter`，G1 裁决 #7）在发布的 SDK 里怎么放 | 移进 `mica_md::compat` 模块并在文档里标「Mica 兼容入口」，门面改从那里 re-export；`public_api.rs` 钉死移动后的路径 |

## 任务

### T5.1 crate 元数据（`crates/md-core/Cargo.toml`）

```toml
description = "Markdown ⇄ block-document engine: CommonMark 0.31.2 (641/641) + GFM (24/24) + footnotes, front matter and Pandoc math, with a round-trip guarantee."
readme = "README.md"
repository = "<Q5 决定>"
documentation = "https://docs.rs/mica-md"
keywords = ["markdown", "commonmark", "gfm", "editor", "blocks"]
categories = ["parser-implementations", "text-processing"]
include = ["src/**/*", "README.md", "SCHEMA.md", "CHANGELOG.md", "LICENSE-*", "examples/**/*"]
publish = false          # 保持 false；T5.12 发布时才改

[package.metadata.docs.rs]
features = ["render"]
```

- `src/lib.rs`：`#![warn(missing_docs)]` 升级为 `#![deny(missing_docs)]`。
- `cargo package -p mica-md --list` 的输出检查：不包含测试大文件（fixtures 留在仓库，不进包）；包体积写进执行记录。

### T5.2 README（英文，`crates/md-core/README.md`）

章节固定：

1. What it is（一段话：Markdown ⇄ 扁平块文档，往返稳定）
2. Conformance（CommonMark 641/641、GFM 24/24，链接到记分牌测试；方言列表）
3. Quick start（`parse_body` / `serialize_body` / `parse_inline` / `render_html` 各一个片段，与 doctest 同源）
4. The document model（链接 SCHEMA.md；UTF-16 偏移说明——这一点对 Rust 使用者反直觉，要醒目）
5. Feature flags（`uuid` 默认开、`render` 默认关及其依赖体积）
6. Guarantees（往返不变量 I1–I8 的英文版；HTML 净化保证：`Export` / `Clipboard` 两个 profile 会去掉什么）
7. Bindings（wasm / npm；FFI 由使用者自行桥接）
8. MSRV（= workspace `rust-version`）、License

### T5.3 Schema 规范（`crates/md-core/SCHEMA.md`，英文）

- 内容 = 02 §6 的英文版 + `schema.rs` 常量表，外加**版本策略**：
  - `SCHEMA_VERSION` 只在「旧文档按新规则读会改变含义」时递增；
  - 新增 kind / data 键 / mark type 不递增，但要写 CHANGELOG；
  - 读端必须忽略未知键与未知 kind（现有导出器行为）。
- `md-core/tests/schema_doc_sync.rs`：解析 SCHEMA.md 里的表格，断言与 `schema::kinds::ALL` 等常量一一对应（文档与代码不会漂移）。

### T5.4 示例（`crates/md-core/examples/`）

| 文件 | 演示 |
| --- | --- |
| `parse_and_serialize.rs` | Markdown → 块 → Markdown，打印往返是否一致 |
| `render_html.rs` | 两种 profile 的 HTML 输出对比 |
| `table_cells.rs` | `decode_cell` / `encode_cell`，含字面 `*` 的转义 |
| `inline.rs` | `parse_inline`，打印 UTF-16 偏移并用 `str::encode_utf16` 验证 |

每个示例在门禁里跑一遍：`for e in …; do cargo run -q -p mica-md --example $e >/dev/null; done`（加进 `scripts/local-ci.sh` 的 rust 部分）。

### T5.5 公开 API 稳定性

- `md-core/tests/public_api.rs`：与 S1 T1.3 的 `facade_surface.rs` 同样的「签名钉死」写法，覆盖 `mica_md` 的**全部** pub 项。
  任何签名变化都会编译失败，迫使改动者同时改这个文件——审核时一眼可见。
- `crates/md-core/CHANGELOG.md`（Keep a Changelog 格式）：从 `0.1.0` 开始。写明什么算破坏性变更：
  1. 公开签名变化；
  2. 删除 / 改名 schema 项；
  3. **同一输入的 `serialize` 输出变化**（往返是卖点，输出稳定性等同 API）——除非是修 bug，并在 CHANGELOG 的 Fixed 里写明输入输出示例。

### T5.6 健壮性与安全

- 把现有 `proptest_parse.rs` 的「永不 panic」属性扩展到新 API：`parse_inline`、`decode_cell`（随机 JSON 值）、`wire::*`（随机字符串当 JSON 输入）。
  文件 `md-core/tests/never_panic.rs`，用例数与现有文件一致（保持门禁速度）。
- README 的 Guarantees 一节写清 HTML 净化的边界：`tagfilter` 列表、`safe_url` 规则、被剥掉的属性（`on*`、`<meta http-equiv=refresh>` 等），
  与 `SAFE_HTML_CSP` 的关系（CSP 是 Mica 门面的东西，SDK 使用者需自行设置）。

### T5.7 npm 包（依赖 S2 的 `crates/md-wasm`）

```
packages/mica-md-js/
  package.json        name（Q2）、version 与 crate 同步、"type": "module"、exports（browser → web 产物，node → nodejs 产物）
  index.js            手写薄封装：把 JSON 字符串接口包成对象接口（parseBody(md) → Block[] 等），并做一次性 init
  index.d.ts          手写类型（Block / Mark / InlineText / HtmlProfile），与 SCHEMA.md 对应
  README.md           英文：安装、init、示例
  test/basic.test.mjs node --test：parseBody / serializeBody / renderHtml 各一例，含 UTF-16 偏移断言
```

- 构建脚本 `scripts/build-md-npm.sh`：同一个 `crates/md-wasm`，`wasm-bindgen --target web` 与 `--target nodejs` 各出一份到 `packages/mica-md-js/dist/{web,node}/`
  （**不提交** dist，`.gitignore`；`check-ignored-sources.sh` 允许名单加上）。
- `npm pack --dry-run` 的文件清单写进执行记录。
- 注意：Flutter Web 用的是 `--target no-modules` 产物（S2），npm 用 `web` / `nodejs`。三者同源同版本，只是 JS 包装形式不同。

### T5.8 许可证审计

列出 `mica-md` 在默认特性与 `render` 特性下的全部传递依赖及其许可证（`cargo tree -p mica-md -e normal --prefix none` 配合 crates.io 元数据，
或现有 `.cargo/audit.toml` 的工具链），写进 `crates/md-core/README.md` 的 License 一节或单独的 `THIRD_PARTY.md`。
`merman` / `ratex-*` 的许可证必须与 `MIT OR Apache-2.0` 兼容，否则 `render` 特性在 README 里单独说明。

### T5.9 本地检查命令与 CI

- `justfile` 新 recipe：

  ```
  [doc("SDK readiness: package list, docs, examples, npm pack (no publish)")]
  sdk-check:
      bash scripts/sdk-check.sh
  ```

- `scripts/sdk-check.sh`：`cargo package -p mica-md --allow-dirty --list`、`cargo publish -p mica-md --dry-run --allow-dirty`
  （`publish = false` 时 dry-run 会拒绝——脚本里临时用 `--config` 覆盖或跳过并提示，执行者选一种并写明）、
  `RUSTDOCFLAGS="-D warnings" cargo doc -p mica-md --no-deps --all-features`、跑全部示例、`bash scripts/build-md-npm.sh && (cd packages/mica-md-js && npm test && npm pack --dry-run)`。
- `.github/workflows/sdk.yml`：`workflow_dispatch` 触发（**仓库现行规定：所有 workflow 只能手动触发**），步骤同 `sdk-check.sh`。

### T5.10 文档站点（可选，低优先级）

docs.rs 会自动生成 Rust 文档；不另建站点。README 里给出 docs.rs 链接即可。

### T5.11 Dart / Flutter 包：本次不发（Q3 推荐）

理由写进 README 的 Bindings 一节：Dart 侧依赖 flutter_rust_bridge 生成代码 + 每个平台的原生库分发（Windows DLL、macOS dylib、Linux so、Android/iOS），
发布与维护成本远高于 Rust crate 和 npm 包；没有外部需求。Mica 自己的 `lib/markdown/` 门面可以作为日后封装 Dart 包的起点。

### T5.12 发布手册（用户批准后执行）

写进本文末尾「发布手册」一节，步骤：

1. 确认 Q1–Q5 的答案已落实到元数据；`just sdk-check` 全绿。
2. `crates/md-core/Cargo.toml`：`publish = false` → 删除该行；CHANGELOG 写 `0.1.0` 日期。
3. 打 tag `mica-md-v0.1.0`（与 App 的 `vX.Y.Z` tag 区分）。
4. `cargo publish -p mica-md`（需要用户的 crates.io token，**由用户本人在自己机器上执行**，token 不进仓库、不进 CI 日志）。
5. `cd packages/mica-md-js && npm publish --access public`（同上，用户执行）。
6. 发布后：README 加 crates.io / npm 徽章；`mica-markdown` 门面依赖改为版本号 + path（`mica-md = { version = "0.1", path = "../md-core" }`）。

## 验收

- [x] `just sdk-check` 全绿（发布动作本身除外）。
- [x] `cargo doc -p mica-md --no-deps --all-features` 零警告；`#![deny(missing_docs)]` 生效。
- [x] `public_api.rs`、`schema_doc_sync.rs`、`never_panic.rs` 通过，且各自证明过能失败。
- [x] 四个示例在门禁中运行。
- [x] npm 包 `npm test` 通过，`npm pack --dry-run` 清单合理。
- [x] 许可证审计完成。
- [x] 「待用户决定」表的答案已记录；未获批准前 `publish = false` 保持不变。

## 执行记录

执行方：Agent D（S5 实际由 D 代 R/W 执行；R/W 的原计划任务在 S2 后已大部分由协调者完成，剩余项见下）。分支 `ss/r/t5-sdk`（自 `single-source` @ `81adf6df`）。
提交全部 `--no-verify`（code-review-graph 钩子本机持续 `database is locked`，S3/S4 已有先例）。

### 待用户决定（答案，2026-10-28 用户拍板）

| # | 问题 | 决定 |
| --- | --- | --- |
| Q1 | crate 名 | **`mica-md`**（crates.io 未占用，已查） |
| Q2 | npm 包名 | **`mica-md`**（无 scope；npm 未占用，已查；`@mica` scope 也空着但不需要） |
| Q3 | Dart / Flutter 包 | **不发**（README Bindings 节已写明理由） |
| Q4 | 许可证 | 沿用 `MIT OR Apache-2.0` |
| Q5 | 托管 | 留 monorepo；`repository` 指向 `https://github.com/weironz/mica` |
| Q6 | 兼容入口 | **移 `mica_md::compat` 模块**，文档标 Mica 兼容入口；门面 re-export 改走 compat；`public_api.rs` 钉死新旧两套路径 |

### 交付明细

| 任务 | 提交 | 内容 |
| --- | --- | --- |
| T5.1 元数据 | `30170178` | description/readme/repository/documentation/keywords/categories/include/docs.rs 元数据；`#![deny(missing_docs)]`；包 32 文件（src + manifest + 8 文档/示例） |
| T5.2 README | 同上 | 8 个固定章节；**HTML 净化边界按引擎实际行为写**（`on*`/URL/tagfilter 剥除；type-1 raw 块为往返保真原样输出，CSP 是使用者的防线——初稿写过头了，测试暴露后修正） |
| T5.3 SCHEMA.md | 同上 | 规范化 schema（英文）+ 版本策略；`schema_doc_sync.rs` 解析表格钉死 `schema.rs` 常量，**已证可失败**（改一个 kind 名即红） |
| T5.4 示例 | 同上 | `parse_and_serialize`（二趟字节稳定 = 往返保证的正确表述）/ `render_html`（两 profile 对比）/ `table_cells`（字面 `*` 转义）/ `inline`（UTF-16 vs 字节偏移，CJK 造出真实分歧）；接入 `local-ci.sh` rust 段 |
| T5.5 API 稳定性 | 同上 | `public_api.rs` 6 用例钉死全部 pub 项（含 compat 与 wire）；`CHANGELOG.md` 定义三类破坏性变更（签名 / schema 项删除改名 / **serialize 输出变化**） |
| T5.6 健壮性 | 同上 | `never_panic.rs`：`parse_inline`、`decode_cell`（随机 JSON）、`wire::*`（随机字符串）各 256 例；**已证可失败**（注入 panic 即红） |
| T5.7 npm 包 | 同上 | `packages/mica-md-js`：`index.js`（JSON 线格式的对象包装，无任何语法逻辑）、手写 `index.d.ts`、README、6 个 `node --test` 用例；`build-md-npm.sh` 出 web+nodejs 两份（同一 md-wasm crate）；nodejs 胶水是 CommonJS，包是 ESM——dist/node 放 `{"type":"commonjs"}` 标记；`npm pack` 13 文件 412.2 kB |
| T5.8 许可证审计 | 同上 | `THIRD_PARTY.md`：默认特性 5 个直接依赖全宽松；`render` 加 merman（MIT OR Apache-2.0）+ ratex-*（MIT）；无 copyleft |
| T5.9 检查命令 | 同上 | `just sdk-check` → `scripts/sdk-check.sh`（包清单白名单、`-D warnings` 文档、三个守门测试、四个示例、npm 构建+测试+pack dry-run）**ALL GREEN**；`sdk.yml` workflow（仅 workflow_dispatch） |
| T5.10 文档站点 | — | 按计划不建站，README 给 docs.rs 链接 |
| T5.11 Dart 包 | — | 按计划不发，理由入 README Bindings |
| T5.12 发布手册 | （本提交） | 见下节 |

### 关键取舍与发现

1. **`cargo publish --dry-run` 不进 sdk-check**：`publish = false` 时它直接拒绝；为 dry-run 临时翻开关有半决定状态入库的风险。`cargo package --list` 的白名单检查覆盖同一面（会打进包什么），不碰开关。执行者选择写明而非绕过。
2. **README 的净化保证初稿写错**：写了「scripts 被剥除」，但引擎对 type-1 raw HTML 块（`<script>`/`<style>`/`<pre>`/`<textarea>` 行首块）**原样输出**——这是 CommonMark 往返保真的刻意行为，防线是 share 页的 CSP。npm 测试用 `<script>` 输入立刻暴露了这一点；README、npm 测试、`render_html` 示例三方对齐为真实语义。
3. **包内必须带 `build.rs`**：打包验证会在干净目录重编译，`MICA_MD_SOURCE_HASH` 是 build.rs 注入的；漏了它包验证失败（这就是 `include` 白名单里有它的原因）。
4. **`inline` 示例的第一版切错了对象**：mark 偏移在**解析后的干净文本**上，不在源串上。修正后的示例用 CJK 展示 UTF-16 与字节偏移的真实分歧（`bold [23..26)` = byte `[27..30)`）。
5. **`serializeBody` 不补尾换行**、**围栏里的 `<script>` 是转义的 code_block**——两处 npm 测试期望初稿写错，按引擎实际行为修正（Rust 为权威）。
6. `deny(missing_docs)` 升级时只发现一个真问题：`[src]` 被当 rustdoc 链接解析，改反引号。

### 门禁（`bash scripts/local-ci.sh all`，2026-10-29）

第三次运行 **ALL GREEN，EXIT=0**（前两次分别死于 fmt 与 clippy，修后重跑）。回执：

| 段 | 结果 |
| --- | --- |
| `rust: fmt --check` / `clippy -D warnings` / tests | 通过；`receipt: 1154 passed (floor 500)` |
| mica-core 门控 SQLite store | `receipt: 213 passed (floor 1)` |
| mica-md 示例 ×4 | 全部运行（parse_and_serialize / render_html / table_cells / inline） |
| wire gold 回执 | `receipt: 9 passed (floor 9)` |
| wasm 体积 | `wasm=200311 B JS=2744 B total=203055 B`（硬预算 262144 B） |
| `flutter: analyze (--no-fatal-infos)` | 0 error / 0 warning |
| flutter 单测（FFI 库重建后） | `receipt: 2051 passed (floor 1000)` |
| `flutter: web compiles` | `√ Built build\web`（156.3s） |
| `mirror.sh` | OK — 263 文件与安装引擎一致 |

`just sdk-check` 单独跑过 **ALL GREEN**（含 cargo doc `-D warnings --all-features` 零警告）。

## 发布手册（T5.12；用户批准后执行，不自动）

> 状态：**未获批，`publish = false` 保持不变**。以下是获批后的一串步骤，一次做完。

1. **确认**：Q1–Q6 的答案已体现在 `crates/md-core/Cargo.toml`（已做）；`just sdk-check` 全绿（已验证）；CHANGELOG 把 `0.1.0` 从 Unreleased 移出并写日期。
2. **翻开关**：删除 `crates/md-core/Cargo.toml` 的 `publish = false` 行。
3. **打 tag**：`mica-md-v0.1.0`（与 App 的 `vX.Y.Z` 区分；发布文档已记录此命名）。
4. **cargo publish**：`cargo publish -p mica-md`。**由用户本人在自己机器上执行**——crates.io token 不进仓库、不进 CI 日志。
5. **npm publish**：先 `bash scripts/build-md-npm.sh`（dist 是生成物），然后 `npm publish --access public`（在 `packages/mica-md-js/`；需要 npm 登录，同样用户执行）。
6. **收尾**：README 加 crates.io / npm 徽章；`crates/markdown` 门面依赖改 `mica-md = { version = "0.1", path = "../md-core" }`（发布后本地仍用 path 开发，版本号开始有约束力）；npm 包 `package.json` 的 `version` 此后随 crate 手工同步（两处，发版时一起改）。
