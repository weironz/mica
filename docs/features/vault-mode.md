# Vault 模式(本地 Markdown 文件)—— 决策 + 分档

> 2026-07-09 定。源于「Mica 能不能像 Obsidian 一样,笔记直接是本地 `.md`?」。
> 调研了 Logseq / SiYuan / Obsidian / Relay 的真实源码(见会话记录),结论如下。

## 现状(非 vault)

离线模式的真相源是**一个 SQLite 库**(`crates/mica-core/src/store.rs` `LocalStore`,`%APPDATA%/mica/local/store.db`):每篇文档存成 `doc_snapshot(doc_id, state BLOB)`,`state` = yrs CRDT 二进制(block model),**不是 `.md`**。Markdown 只是导入/导出格式。这是 Notion/AppFlowy 型的 DB-first 架构,不是 Obsidian 型的 files-first。

## 核心结论

**纯 `.md` 文件当真相源 ⨯ yrs 实时 CRDT 协同 = 真硬冲突。** 前人一致证明:一旦「一个 `.md`」当同步单元,冲突就退化到整文件 LWW + 冲突副本(`.md` 快照丢掉了 CRDT 收敛所需的 op 历史)。所有 files-first 产品(Obsidian、Logseq file-graph)都**刻意不做**实时协同;Logseq 为拿到协同,干脆**另起一套 SQLite db-graph**。

→ **不追求「文件真相源 + 实时协同」同时成立。** vault 模式刻意把实时 CRDT 协同留给云端/DB 模式当差异化;分三档增量做,越往后越重、越战略。

Mica 的独门优势正好压中最贵的那块:`crates/markdown` 已是 **CommonMark 100% + GFM、round-trip 定点引擎**(`export(import(x))` 不变量)。SiYuan 为此自研了 Lute;Mica 白捡。

## 分档

| 档 | 是什么 | 状态 |
|---|---|---|
| **S** | **只读导入**:选文件夹 → 遍历 `.md` → 落进现有 DB store 当文档,目录建页树。不写回、不监听。 | ✅ **已实现**(本分支) |
| **M** | 导出成 `.md` 文件夹 + 可重复 re-sync(进 git);文件监听单向 reconcile。DB 仍是真相。 | ⬜ |
| **L** | 真 vault:`FsVaultStore` 上到 `trait Store` 后,`.md` 就是真相,`.mica/` sidecar 承载非-GFM 属性,双向监听 + echo 抑制。**刻意文件级同步、无实时协同。** | ⬜ 战略下注,非默认 |

## S 档实现(已落地)

- **Rust**:`MicaDocument::from_markdown(markdown)`(`clients/mica_flutter/rust/src/api/document.rs`)—— 用权威引擎 `mica_markdown::import_markdown` 解析,`mica_markdown::Block` 与 `mica_core::Block` 字段一致,零翻译。解析留在 Rust(原则 #2)。带 2 个 Rust 单测。
- **Dart facade**:`LocalOffline.importVaultTree(entries, workspaceId)`(`lib/local/local_offline_io.dart`)—— 吃「已遍历的相对路径 + 字节」树,只留 `.md`、跳过 `.`-dir;文件夹**懒建成空页**(只建真有 `.md` 的祖先),每篇 `fromMarkdown` → `saveDoc` + `saveView`。web 变体为 stub。
- **UI**:复用现有「工作区菜单 → Import → Folder」(选文件夹 + 遍历本就共享);本地把原来 no-op 的 `onImportWorkspaceTreeInto` 接到 `_localImportVaultTree`(`lib/main.dart`),导入后刷新页树 + snackbar。
- **测试**:`integration_test/vault_import_test.dart` 真机过 —— 合成树 → 导入 → 断言 docs + 文件夹页 + 父子 + 内容 round-trip;`document.rs` 两个 Rust 单测。

**边界(S 档刻意不做)**:不写回源文件夹、不监听外部改动、不给块 id(双链留到 M/L 走 `.mica/` sidecar,不学 Logseq 往 `.md` 塞 `id::`)、不追求非-GFM 方言在纯 `.md` 里无损。
