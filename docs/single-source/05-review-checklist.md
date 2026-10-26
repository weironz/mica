# 05 审核清单

> 审核方在全部阶段完成后逐条核对；执行方在每个阶段结束时用对应小节自检。
> 每一条要么附「怎么验证的」（命令 + 输出摘要），要么附「为什么不适用」。**不接受「看起来没问题」**。

## A. 结构性检查（命令可复核）

| # | 检查 | 命令 / 方法 | 期望 |
| --- | --- | --- | --- |
| A1 | Dart 里没有 Markdown 语法实现 | `grep -rnE "parseInline\(|inlineToMarkdown\(|markdownToBlocks\(|escapeInline\(|escapeBlockLeader|_rangeMarkdown|markdownListLayout|tableToMarkdown" clients/mica_flutter/lib` | 无结果 |
| A2 | 镜像文件已删除 | `ls clients/mica_flutter/lib/editor/{markdown,list_serialization}.dart` | 不存在 |
| A3 | `marks.dart` 只剩模型 | `wc -l clients/mica_flutter/lib/editor/marks.dart` + 人工读一遍 | ≤ 400 行，无解析/序列化函数 |
| A4 | Dart 只经门面调用引擎 | `grep -rn "mdParse\|mdSerialize\|mdRender\|micaMd" clients/mica_flutter/lib \| grep -v "lib/markdown/"` | 无结果 |
| A5 | 核心 crate 依赖面 | `cargo tree -p mica-md -e normal --depth 1` 与 `--no-default-features` 各一次 | 只有 serde / serde_json / thiserror（+ uuid、render 依赖按特性） |
| A6 | 消费方零改动（S1） | `git diff <S1 起点>..<S1 终点> -- crates/{app-core,api-server,interchange,mcp-server,mica-core} clients/mica_flutter/rust/src` | 空（S2 起 FFI crate 的新增文件除外） |
| A7 | wasm 产物未提交 | `git ls-files clients/mica_flutter/web/mica_md clients/mica_flutter/test/support/mica_md packages/mica-md-js/dist` | 空 |
| A8 | 行数棘轮 | `bash scripts/check-file-size.sh` | passed；预算表只减不增（`git log -p scripts/file-size-budget.txt`） |
| A9 | 依赖豁免已登记 | 读 `AGENTS.md` 依赖清单 | 有 wasm-bindgen 一行，范围限定 `crates/md-wasm` |
| A10 | AGENTS.md Markdown 规则已更新 | 读 `AGENTS.md` | 是 S4 T4.12 的新规则原文 |

## B. 测试与门禁

| # | 检查 | 期望 |
| --- | --- | --- |
| B1 | `bash scripts/local-ci.sh all` | 全绿；Rust、Flutter 回执（`receipt:`）数字合理（Flutter 用例总数下降应能由删除的 A 类测试解释） |
| B2 | 浏览器套件 | `scripts/browser-tests.sh` 自动发现并通过契约测试 `markdown_codec_contract_web_test.dart` |
| B3 | Windows 集成测试 | `integration_test/markdown_codec_test.dart` 在 Windows 通过（证据） |
| B4 | 契约黄金文件 | `cargo test -p mica-md --test wire_gold` 通过；黄金文件的每次变化都在 PR 里有解释 |
| B5 | 记分牌 | CommonMark 641/641、GFM 24/24 |
| B6 | 新测试都证明过会失败 | 每个阶段执行记录里有「如何证明会失败」的说明（I1–I8、facade_surface、public_api、compat、roundtrip 等） |
| B7 | 剪贴板往返 | `clipboard_mica_roundtrip_test` 在门禁中；例外清单每一项有理由 |
| B8 | 存储兼容 | `table_cell_storage_compat_test` 三项通过 |
| B9 | 迁移的测试有去处 | S3/S4 执行记录里，每个删除的 Dart A 类测试都有对应的 Rust 测试名；抽查 5 个，确认 Rust 测试确实覆盖同样输入 |

## C. 行为

| # | 检查 | 期望 |
| --- | --- | --- |
| C1 | S3 T3.7 分诊表 | 每条差异有结论；B 类全部修复且有回归测试；R 类中用户可见的已列入「用户可见变化」 |
| C2 | 「待用户决定」 | 汇总给用户，**审核方不替用户拍板** |
| C3 | 降级 | `UnavailableMarkdownCodec` 下：粘贴退回纯文本、复制只有 text/plain、表格旧字符串单元格原文显示——各有一个测试 |
| C4 | 真实路径证据 | S3 T3.10、S4 T4.11 的 Windows 截图与 Web Playwright 截图齐全；审核方至少亲自复现其中 3 个场景 |
| C5 | AI 编辑往返 | AI「读取当前页」发出的 Markdown 与服务端解析结果一致（同一引擎），抽查 3 个文档 |

## D. 性能（对照 02 §7 与 S0 基线）

| # | 指标 | 期望 |
| --- | --- | --- |
| D1 | 桌面 `initMarkdownCodec` | ≤ 30 ms |
| D2 | Web wasm ready / 首帧推迟 | ≤ 60 ms |
| D3 | wasm 体积 | ≤ 200 KB gzip（构建脚本强制） |
| D4 | 粘贴 / 复制 10k 字符 | ≤ 基线 × 1.2（桌面、Web 分别） |
| D5 | 500 单元格表格首次解码 | 桌面 ≤ 16 ms、Web ≤ 30 ms；重新布局 0 次引擎调用 |

超预算项：执行记录里要有原因分析和审核方 / 用户的处理决定。

## E. 文档

| # | 检查 |
| --- | --- |
| E1 | 各阶段执行记录完整（做了什么、偏离、回执、证据、数字） |
| E2 | 03 §5 的文档全部更新；`docs/README.md` 索引正确 |
| E3 | `docs/plans/roadmap.md` 本专项条目已移入 `roadmap-done.md`（发版时） |
| E4 | `crates/md-core/README.md`、`SCHEMA.md`、`CHANGELOG.md` 存在且与代码一致（`schema_doc_sync.rs`） |

## F. 审核方的抽查方法

1. 随机选 3 个已删除的 Dart 镜像函数，读它们最后一个版本的实现，找出 2 个边界行为（例如 CJK 强调、未闭合公式），
   在 Rust 测试里确认这两个行为有覆盖；没有就是缺口。
2. 用一份真实的复杂文档（含公式、脚注、嵌套列表、表格、mermaid、代码块）在桌面和 Web 各做一次「复制 → 粘贴到 Mica → 再复制到 Typora」，
   比较前后。
3. 在 Windows 上用**旧版本**安装包打开**新版本**写过表格的文档（或反过来），确认显示正确（D6 的实地验证）。
4. 读 `crates/md-wasm/src/lib.rs` 和 `clients/mica_flutter/rust/src/api/markdown.rs`：确认都只是一行转发，没有任何逻辑。
