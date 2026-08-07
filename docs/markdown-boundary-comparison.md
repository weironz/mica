# Markdown 引擎放在哪 —— AppFlowy / AFFiNE 源码级对照

> **2026-08-07 调研。** 起因是一个问题:「Mica 的 markdown 解析两端都有,如果拿去给别人调用是不是很难」。
> 按 CLAUDE.md 第 6 条,拍板前先扒同类产品 —— 重点不是「它支不支持」,而是**相同约束下它具体怎么做、
> 又刻意没用什么**,以及**验证自己脑子里那条「必须 X 才能 Y」的前提**。
>
> 结果推翻了一条前提,见 §3。
>
> 依据的源码版本(GitHub,调研当天):AppFlowy `5cf3a36`、AppFlowy-Web `04da5de`、AFFiNE `921e83b`(v0.27.0)。
> 引用的路径都是当时实际读到的文件,不是记忆。

## 1. 一张表

| | markdown 解析在哪 | 用现成库还是自研 | 服务端参与吗 | 编辑器抽包了吗 |
| --- | --- | --- | --- | --- |
| **AppFlowy** | 客户端,**两套**(Flutter 一套、Web 一套) | **现成**:Dart 用 pub `markdown`,Web 用 `remark`/`unified` | **不参与** | 是,`appflowy_editor` 在 pub.dev |
| **AFFiNE** | 客户端,**一套**(`MarkdownAdapter`,基于 mdast) | **现成**:remark 生态 | **不参与** | 是,`@blocksuite/*` 在 npm |
| **Mica** | **两端各一份** | **全自研** | **参与** | 否 |

Mica 的量:Rust `crates/markdown` 6548 行(`lib.rs` 5981 + `properties.rs` 567);Dart 镜像 3763 行
(`editor/markdown.dart` 1477 + `marks.dart` 2016 + `table.dart` 270);`editor/` 整体约 26k 行。

## 2. 证据

### AppFlowy:Rust 侧没有 markdown ↔ 文档模型

`frontend/rust-lib` 全仓搜 markdown,只有三类命中,没有一类是解析成块:

- `flowy-ai/src/ai_tool/markdown.rs` —— 读 md 文件、渲染成 HTML,供 AI 索引用。
- `flowy-folder/src/entities/import.rs`、`flowy-document/src/entities.rs` —— `ImportType::Markdown` /
  `ExportType::Markdown` **枚举值**。
- `flowy-core/src/deps_resolve/folder_deps/folder_deps_doc_impl.rs` —— 那条 Rust 导入路径是
  **`// TODO(lucas): import file from local markdown file` + `Ok(())` 空实现**。

真正的转换在 Dart,而且**不是自研解析器**:

```dart
// frontend/appflowy_flutter/lib/plugins/document/presentation/editor_plugins/parsers/markdown_code_parser.dart
import 'package:markdown/markdown.dart' as md;
class MarkdownCodeBlockParser extends CustomMarkdownParser { … }
```

pub 的 `markdown` 包负责 CommonMark,AppFlowy 只写两侧**适配层**:读侧 `CustomMarkdownParser`
(md AST → 自家 Node),写侧 `NodeParser`(自家 Node → markdown 字符串,如
`callout_node_parser.dart` / `simple_table_node_parser.dart`)。入口是
`lib/shared/markdown_to_document.dart` 的 `markdownToDocument`。

### AppFlowy:web 是另一套编辑器,不是 Flutter web

`frontend/` 下只有 `appflowy_flutter` 和 `rust-lib`,**没有 web**。AppFlowy Web 是独立仓库
`AppFlowy-IO/AppFlowy-Web`,`package.json` 是 React 应用:

- `@appflowyinc/editor` ^0.1.13 —— 另一个编辑器包
- `slate` / `slate-react` / `@slate-yjs/core` —— 第三套编辑器栈
- `remark-parse` / `remark-gfm` / `unified` / `hast-util-to-mdast` —— 另一套 markdown 适配
- `yjs` 14

所以 AppFlowy 是**两套完整编辑器 + 两套 markdown 适配层**,跨语言又跨框架。

### AFFiNE:一份适配器,服务端不碰

`blocksuite/affine/shared/src/adapters/markdown/markdown.ts` 里的
`MarkdownAdapter extends BaseAdapter<Markdown>`,输入是 `MarkdownAST`(mdast,remark 生态),
输出是 `BlockSnapshot`。调用方全在客户端:编辑器
(`packages/frontend/core/src/blocksuite/utils/markdown-utils.ts`)、AI、评论快照、
Obsidian/Bear 导入(`blocksuite/affine/widgets/linked-doc/src/transformers/`)、
`packages/common/reader`(被 `@affine/nbstore` 依赖)。

**服务端不依赖它**:`packages/backend/server/package.json` 里没有任何 `@blocksuite/*`,
只有 `yjs ^13.6.27` 和 `@affine/server-native`。

AFFiNE 的服务端 Rust(`packages/backend/native/Cargo.toml`,crate `affine_server_native`)放的是
**CRDT + 重解析 + 加密**:`y-octo`(自研 Rust Yjs)、`doc_extractor`、`affine_doc_loader`、
`aes-gcm`/`p256`/`hkdf`、`image`/`matroska`/`mp4parse`/`little_exif`、`sqlx`、`llm_adapter`、
`tiktoken-rs`。**markdown 不在里面。**

这和 Mica「Rust-first 数据面」的选择高度一致 —— 差别恰恰在 markdown 这一项。

### 抽包的真实程度

- `appflowy_editor` 确实在 pub.dev,发布者 AppFlowy.io,最新 **6.2.0**。主应用
  `frontend/appflowy_flutter/pubspec.yaml` 里就是一条普通依赖。
  ⚠️ 但 6.2.0 是**约 8 个月前**发布的,而主应用一直在动 —— 抽出去的包滞后于自家应用,
  是抽包的经典结局,记在这里当代价。
- BlockSuite 是 npm 上的 `@blocksuite/*`,在 AFFiNE monorepo 的 `blocksuite/` 目录里开发。

## 3. 被推翻的前提

**原本的说法**:「web 端没有 FFI/WASM,所以 markdown 解析必须在 Dart 侧再写一份。」

**前提本身没错,但它不是根因。** 根因是 Mica 选择了**服务端也要能解析 markdown**:

- MCP 写入(`mica_update_document` 的 append / insert_at / replace_all)
- ZIP 导入导出、Notion 导入
- 服务端 HTML / PDF 导出

如果 markdown 只活在客户端(AppFlowy 的 Flutter 端就是这样),**一份 Dart 就够了**,
`crates/markdown` 那 6548 行根本不必存在。

所以真正该问的不是「web 有没有 FFI」,而是「**服务端解析 markdown 这件事,值不值那份重复**」。

**判断:值。** AppFlowy 那个 `TODO(lucas)` 空实现恰好说明他们没做这件事。而 MCP 写入是 Mica 的
差异化功能 —— 它必须在服务端把 markdown 变成 block ops,不能要求调用方先跑一个 Flutter 客户端。
这条能力是 Mica 主动选的,重复是它明码标价的成本,不是疏忽。

## 4. 两个反直觉的发现

**① Mica 的「双表示」代价比 AppFlowy 小。**

AppFlowy 是两套编辑器(Flutter + React/Slate)+ 两套 markdown 适配层,跨语言跨框架。
Mica 是**一套编辑器**(Flutter 同构,web 与桌面同一份 UI)+ 两份 markdown 引擎。
按「要同步的东西有多少」算,Mica 少一整个编辑器。

**② 真正独有的成本是「自研 CommonMark 引擎」,不是「两端都有」。**

两家都没写解析器,只写 adapter。Mica 两端全是手写的 —— 这是 in-house 原则最贵的一笔。
同时它也是 Mica 独有的资产:记分牌(`docs/commonmark-scoreboard.md`,读侧 641/641 + GFM 24/24)、
round-trip 不变量、方言完全可控(脚注 / front matter / Pandoc 数学 / callout / details)。
用现成库拿不到这些 —— 但也要诚实:**这些资产只有在「markdown 保真是产品卖点」时才值钱**,
而这正好是 Mica 的定位。

## 5. 「拿去给别人调用」——修正后的判断

两家的答案一致,而且和直觉相反:

> **他们抽出去的是编辑器 + 块模型;markdown 只是里面的一个 adapter,从来不是独立产品。**

Mica 恰好倒过来:

| 层 | 可抽性 | 为什么 |
| --- | --- | --- |
| `crates/markdown`(Rust) | **高** | 叶子 crate,依赖只有 serde / serde_json / thiserror / uuid(`render` feature 关掉时连 merman/ratex 都不拉);反过来是 api-server / app-core / interchange / mcp-server / mica-core / Flutter FFI **六处**依赖它 |
| Dart 镜像 | 低 | 服务编辑器热路径,和 `highlight.dart` / `table.dart` 缠在一起;而且它按定义要跟 Rust 同步 —— 只拿镜像不拿权威,等于把同步问题送给别人 |
| 编辑器(`editor/`) | **最低** | 26k 行,直接 import `l10n/locale_controller.dart` / `ui/theme_tokens.dart` / `diagnostics.dart` / `cjk_fonts.dart` / FFI `render.dart`,还有整套 `_stub`/`_web` 条件导入变体 |

### 什么条件下值得把 `crates/markdown` 发布出去

**先决条件(缺一条就先别发)**:

1. **愿意把块模型当成公开契约。** 它的 API 不是 `markdown → markdown`,而是
   `markdown ↔ DocumentSnapshotPayload`。一旦发布,`data.indent` / `data.quote` / `data.li`
   这些今天的**内部约定**就变成别人的 API,再改就是 breaking change。这是最贵的一条,
   而且它不可逆。
2. **接受「它是方言不是 CommonMark 库」的定位,并在 README 第一句说清楚。** 否则会持续收到
   「为什么不支持 X」的 issue,而 X 恰恰是刻意不做的。
3. **写侧规范化子集要有明确文档。** 读 641/641 是可宣传的,写侧输出的是子集 —— 不说清楚就是
   在制造预期落差。

**不成为理由的**:「代码已经很干净了」「反正是叶子 crate」—— 可抽性是必要条件,不是充分条件。
上面第 1 条才是决策点。

**当前结论(2026-08-07):暂不发布。** 没有外部需求驱动,而第 1 条的代价是永久的。
真要做,顺序是:先冻结块模型的公开面 → 补写侧文档 → 再谈发布。

**编辑器那层建议不动。** 它的价值在于跟这套块模型和 CRDT 同步紧密咬合;拆开之后剩下的部分
未必比现成的 Flutter 富文本方案更有优势 —— 而 AppFlowy 那个滞后 8 个月的 pub 包,正是
「抽出去之后要单独养」的价签。
