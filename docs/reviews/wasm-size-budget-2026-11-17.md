# Markdown 引擎 wasm 体积预算（2026-11-17）

> 结论：`scripts/build-md-wasm.sh` 的硬预算由 256 KiB（262,144 B）调到 **288 KiB（294,912 B）**，用户决定。
> 起因是代码高亮器从 Dart 移入 Rust 引擎（gh#178）。预算保留，作用是拦住"不知不觉的膨胀"，不是给引擎定一个永久的"合理大小"。

## 1. 场景

gh#178：分享页和 HTML 导出的代码块没有颜色，编辑器里却有。原因是高亮器（分词、44 种语法、语言识别）只在 Dart 里有一份（`lib/editor/highlight.dart`，2,435 行），而导出页和分享页由 Rust 引擎渲染。

用户选了方案 1：把高亮器移进 `crates/md-core/src/highlight`，编辑器（桌面 FFI、Web wasm）、HTML 导出和分享页共用这一份。对拍夹具从原 Dart 实现录制，包含 3,805 条分词用例和 177 条检测用例，结果完全一致。

代价是 Web 端的 Markdown 引擎 wasm 变大，超出了 256 KiB 硬预算。

## 2. 实测（Linux，`gzip -n -9`，wasm 与 JS 分别压缩后相加）

| 版本 | md 引擎包 | 与基线差 |
|---|---|---|
| 迁移前 | 250,157 B | — |
| 直接实现：`regex-lite` + serde 派生解析 | 301,333 B | +51 KB |
| 优化后：自写 JS 子集正则匹配器（原生支持前瞻、回溯栈在堆上）、手工遍历 JSON、语言数据紧凑存放 | **271,328 B** | **+21 KB** |

| 首屏其他资源 | gzip 后 |
|---|---|
| `main.dart.js` | 1,474,608 → 1,464,185 B（**−10.4 KB**，Dart 高亮器删掉了） |
| CanvasKit（`canvaskit.wasm`） | 约 2.9 MB |
| CRDT 文档引擎 wasm（`crates/doc-wasm`） | 约 250 KB（有独立的 256 KiB 预算，不受本次影响） |

净增约 **10.7 KB**，不到网页版首次下载量的 0.3%。

被拦下的 30 KB 来自两处，都在 `twiggy` 里定位到：serde 的内部标签枚举（`#[serde(tag = "op")]`）一项就占约 18 KB 未压缩体积；`regex-lite` 本身也不小。这正是预算要拦的那类膨胀。

## 3. 为什么要有这个预算

- 网页版打开文档前，必须先下载并编译这个 wasm，它直接影响首次加载。
- 依赖和派生代码会让体积悄悄变大：一行 `#[derive]` 或一个"小"依赖，就能加几十 KB，代码评审时看不出来。硬预算让每次变大都必须被看见、被决定。
- 来历：S2 阶段定为 200 KB，在 Linux 上实测后用户改为 256 KiB（见 [single-source/reviews.md](../single-source/reviews.md)）。

## 4. 业界做法

- **性能预算**：Lighthouse `budget.json`、`size-limit`、`bundlesize`，webpack 默认在单个资源超过 244 KiB 时告警。预算会随功能调整，但每次调整都是明确的决定，并附上测量数据。
- **按需加载**：不常用的功能用到时才下载，不进首屏。本仓库的公式和 Mermaid 预览已经这样做（`scripts/build-preview-wasm.sh`），不计入启动包。
- **看整页总量，而不是单个文件**：用户感受到的是首屏总下载量。Flutter Web 的大头是 CanvasKit 和 `main.dart.js`。

## 5. 以后怎么调

1. 预算继续是硬失败。调高必须附上：变大前后的实测数字、增量来自哪里（`twiggy top`），以及为什么不能削减或按需加载。记录写进本文件或新文件，并更新 `docs/single-source/02-architecture.md` 的 wasm 体积一行和 `05-review-checklist.md` 的 D3。
2. 新功能如果明显超过几十 KB、又不是首屏必需（例如某种渲染器、某类导入），先考虑按需包，再考虑调预算。
3. 变大之前先找"免费"的削减：serde 派生改成手工遍历 `serde_json::Value`、去掉内部标签枚举、数据紧凑存放、避免引入通用正则或 Unicode 表等重依赖。

## 6. 本次改动位置

- `crates/md-core/src/highlight/{mod.rs, regex.rs, languages.json}`；对拍测试 `crates/md-core/tests/highlight_parity.rs`
- 绑定：`wire::highlight_json`，FFI `md_highlight`，wasm `highlight`
- 导出：`mica_md::compat::export_html_highlighted`，页面样式 `mica_markdown::static_page_css`
- 编辑器：`clients/mica_flutter/lib/editor/highlight.dart`，只剩配色和缓存
- 预算：`scripts/build-md-wasm.sh`、`scripts/test-md-wasm-build.sh`
