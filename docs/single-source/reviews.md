# 审核记录

> 审核方在每个停点（[04 §4.2](04-parallel-plan.md)）的结论写在这里，新的写在最上面。
> 执行方只读本文件：「必须修改」的条目改完后，在对应阶段的执行记录里写明修改位置，再请用户转告审核方复核。

## 记录格式

```
## G1 契约冻结 — <日期> — 通过 / 要求修改
- 审核对象：single-source @ <提交号>，阶段执行记录 <链接>
- 核对结果：逐条列 04 §4.2 该停点「审核方核对什么」的每一项：✅ / ✖ + 证据（命令与输出摘要）
- 必须修改：编号列出，每条写清楚问题和期望（不写怎么改）
- 建议（不阻塞）：
- 交给用户的决定：（G2、G3 才有）
- 结论：通过后可以做什么（例如「S0+S1 可以并回 main」「WP-W 可以开始 T2.2」）
```

<!-- 第一条审核记录写在这一行下面 -->

## G4 最终审核 — 2026-10-29 — 通过（审核方修复 6 处后）

- 审核对象：`single-source` @ `f161e1e0`（S3 收尾 4 个提交 + S4 7 个 + S5 2 个 + 文档），按 05 清单 A–F 逐条核对；
  审核方修复在 `e183c27b` / `473e21a7` / `ccef0112`，main 同步 merge `fed43b31`。
- **门禁（审核方 Linux，Flutter 3.47.1 + Chromium）**：
  - `f161e1e0` 原样：`bash scripts/local-ci.sh all` 跑到 `ALL GREEN`、exit 0——Rust 1157 + 213 passed、wire 黄金 9、
    `flutter analyze` 0 warning、Flutter 2027 passed、Web 构建、Chrome 浏览器套件 9 组全部通过。
  - 合并 main 与审核方修复后的最终树（`fed43b31` + 本记录）：`local-ci.sh all` 再跑到 `ALL GREEN`、exit 0——Rust `1160 passed` + store `213`、wire 黄金 9、wasm 203,003 B、
    analyze 0 warning、Flutter `2030 passed`（多出的 3 个是新增的降级测试）、Web 构建、Chrome 浏览器套件 9 组全部通过。
  - `just sdk-check` ALL GREEN（含新增的 `--no-default-features` 一步、npm 6 个 Node 测试、`npm pack --dry-run`）。

### 逐条结果

| 节 | 结果 |
| --- | --- |
| A1 | ✅ `lib/` 只剩门面接口 `MarkdownCodec.parseInline` 的声明与两平台转发，无 Dart 语法实现 |
| A2 / A3 | ✅ `markdown.dart`、`list_serialization.dart` 已删；`marks.dart` 373 行，只剩标记模型操作 |
| A4 / A7 | ✅ 门面外无引擎直调；wasm / npm 产物未入库 |
| A5 | ✅ `mica-md` 依赖：serde、serde_json、thiserror（默认特性另加 uuid） |
| A6 | ✅ 消费方 crate 只有 `app-core/tests/sync_pg.rs`（与 main 同一修复，合并时取 main） |
| A8 | ✖→✅ **预算被调高**：`dd183384` 把编辑器库预算 7192 → 7232（提交说明却写「收紧」）。审核方压回 7186（见修复 2） |
| A9 / A10 | ✅ `wasm-bindgen` 已登记；AGENTS.md「Markdown 只有一份实现」为 S4 T4.12 原文 |
| B1 / B2 / B4 / B5 / B7 | ✅ 见上方门禁；CommonMark 641/641、GFM 24/24 在 `mica-markdown` 测试中 |
| B3 | ⚠ Windows 集成测试（`markdown_codec_test` 10 passed、`clipboard_roundtrip_test` 1 passed）以执行方回执为准，审核方无 Windows |
| B6 | ✅ 各阶段新测试都有「如何证明会失败」记录；审核方新增的测试同样先证红 |
| B8 | ✅ `table_cell_storage_compat_test` 9 例通过（修复 1 后仍通过，表格 / 单元格相关 65 个测试全绿） |
| B9 | ✖→✅ 抽查 S4 T4.9 映射：7 个 Rust 对应文件与 3 个点名测试都在；但 `inline_scan_perf_test` 9 例只迁了 7 例，`math_run_spans` 遇大量未闭合 `\(` 与「嵌套标记导出」两例没有 Rust 对应——审核方补到 `crates/md-core/tests/inline_perf.rs` |
| C1 / C2 | ✅ 分诊与「待用户决定」（U-1、S4 选项 b、S5 Q1–Q6）都由用户拍板，有记录 |
| C3 | ✖→✅ **三种降级行为没有测试**（只测了「降级门面会抛异常」）。审核方补 `markdown_codec_degradation_test.dart`：粘贴退回纯文本、复制只放纯文本、旧字符串单元格原文显示；三处降级逻辑各自弄坏时测试都变红 |
| C4 | ⚠ 截图在执行方本机（不入库）；Web e2e 与桌面集成测试的回执在 S3 / S4 执行记录。审核方复现：Chrome 浏览器套件全绿、引擎不可用时的粘贴 / 复制 / 表格三场景（C3 测试） |
| C5 | ✅ AI「读取当前页」与服务端解析都走同一引擎；G3 时已修「引擎不可用时当成空页」 |
| D1 / D2 | ✅ 桌面 init 2 ms、Web 初始化 p50 14 ms（M2 记录） |
| D3 | ✅ wasm 202,923–203,055 B ≤ 262,144 B（审核方 Linux 复建 203,003 B） |
| D4 | ✅ 粘贴三档 ≤ Dart × 1.2（M2）；复制 10k 合计桌面约 4.9 ms、Web 4.3 ms ≤ 16 ms。合并时补上基准里缺的 `copy.10k` 合计判定行 |
| D5 | ✅ 桌面 500 单元格首次解码 0.8–1.0 ms（执行方）；**Web 原未测**，审核方 Chrome 实测 5 轮中位 7.3 ms、最大 13.1 ms ≤ 30 ms；重排 0 次引擎调用 |
| E1–E4 | ✅ 执行记录完整；03 §5 所列文档都已更新；`README` / `SCHEMA.md` / `CHANGELOG.md` 存在且由 `schema_doc_sync` 钉住；E3（移入 roadmap-done）按 AGENTS 在发版时做 |
| F1 | 见 B9 |
| F2 / F3 | ⚠ 需要 Windows 和旧版安装包，审核方做不了：真实复杂文档「复制 → 粘贴 → 再复制到 Typora」、旧版本客户端读新表格。存储形态与旧读法的兼容由 T4.8 测试按引擎解码证明 |
| F4 | ✅ `md-wasm/src/lib.rs` 与 FFI `api/markdown.rs` 都是十个一行转发 |

### 审核方直接完成的修复

1. **单元格存储规则只有一处**（`e183c27b`）：S4 选项 b（读回不一致就存 `{text, marks: []}`）原来写在 Dart `TableData._encodeCells` 里，
   Rust `encode_cell` 不知道这条规则，SDK 用户和其他写入方仍会得到有损的字符串。规则移进 `encode_cell`，**I3 恢复为严格无损**；
   Dart 删掉读回比较；proptest 增加无标记单元格一条（先红：最小反例 `" "`）；wire 黄金只在两类有损用例上变化；02 §2.2 / §6.4、`SCHEMA.md`、`CHANGELOG` 同步。
2. **编辑器库行数还回去**（`473e21a7`）：见 A8。三段手写 try/catch 改用 `withEngineOrNull`，表格区域复制的纯文本 + HTML 拼装移进独立库 `table_copy.dart`；
   顺手把被插到 `_handleRichPaste` 文档注释中间的 `_parseOrNull` 注释归位。
3. **补测试**（`ccef0112`）：C3 三种降级、B9 两个性能用例；`public_api.rs` 无条件引用 `BlockIds::Uuid`，
   导致 `cargo test -p mica-md --no-default-features`（wasm 用的特性组合）从 S5 起编译不过——加 `cfg(feature = "uuid")`，并让 `sdk-check.sh` 跑这一组合。
4. **合并 main**（`fed43b31`）：三处冲突都是两边各自做了同一个修复，取法见合并提交说明；基准补上 02 §7 的 `copy.10k ≤ 16 ms` 合计判定。

### 建议（不阻塞）

- SDK 的 Export HTML 对行首 `<script>` 等原样 HTML 块原样输出（为了 Markdown 往返保真），Mica 靠分享页 CSP 防住；SDK 用户没有这层 CSP。
  **对外发布前**建议加一个「严格」档 HTML 输出（或默认剥除），README 已如实写明现状。
- `packages/mica-md-js/package.json` 没有 `"private": true`，发布前的误操作只靠 npm 登录拦着；发布手册已写明由用户本人执行。
- 执行方三次报告的门禁回执都有不实之处（G3 的 fmt、S3 的编辑器预算「收紧」、S5 的 no-default 编译），都被独立复跑抓到——以后任何阶段报全绿前，请把预算表 diff 和 `--no-default-features` 也看一眼。

### 结论

**G4 通过。** `single-source` 用 merge 提交并回 main。不发版：`publish = false` 保持，crate / npm 发布与 App 发版都等用户明确要求；
发版时按 AGENTS 把路线图条目移入 `roadmap-done.md`。

## G3 补充 — 2026-10-28 — 审核方复跑 Flutter，并按用户要求直接完成必须修改

- 审核方装了 Flutter 3.47.1（官方包，SHA-256 校验，Dart 3.13.1 与执行方一致），在 Linux 上复跑 `98c71d14`：
  - ✖ **执行方的 `local-ci.sh all` 全绿回执不成立**：第一步 `cargo fmt --all --check` 就失败（`crates/md-core/tests/wire.rs`，从 B-1 的 `7ef6bbba` 起，`dd183384` 与 `98c71d14` 同样失败）；
    格式修正后 `flutter analyze --no-fatal-infos` 仍有 5 个 warning：`_rangeMarkdown` / `_listRunHtml` / `_tableHtml` 无引用（G3 停点的预期状态，T3.6 删除后消失），
    以及两个测试的多余 import。脚本遇错即停，所以不可能在 `dd183384` 上跑出全绿。上一轮 G3 审核方只跑了测试与 clippy、没跑格式检查，也漏了这一处。
  - ✅ 跳过 analyze 后：Rust 1142 + 213 passed；Flutter 单元测试 `receipt: 2210 passed (floor 1000)`、29 skipped、0 failed（执行方 Windows 2232 / 21，差异是平台条件测试）；
    `browser-tests.sh` 9 组 Chrome 套件全部通过（含真实 wasm 门面契约）。
- 按用户要求，三条必须修改由审核方完成（细节与回归测试见 S3 执行记录末节「G3 必须修改：审核方完成」）。改写往返测试时抓到读回端一个真 bug（字面 `!` 紧挨链接读回成图片），已修并配测试。
- 执行方注意：
  - 三条必须修改已完成，**不要再做**；在本提交之后继续 T3.6 / T3.9–T3.11。
  - T3.6 删掉 `_rangeMarkdown` 等之后，`flutter analyze` 应为 0 warning。以后报门禁回执前，请确认 `local-ci.sh all` 真的跑到了结尾的 `ALL GREEN`。

## G3 删除之前 — 2026-10-28 — 通过（附条件）

- 审核对象：`single-source` @ `98c71d14`（S3 的 `e7b4a533` … `98c71d14`，8 个提交），S3 执行记录「当前结论」至「G3 停点声明」。
- 核对结果：
  - ✅ **停点前提成立**：旧实现都还在——`controller.dart` 的 `_rangeMarkdown`（1345 行）、`selectionHtml` 旧体与 `_listRunHtml`、`marks.dart` 的 `inlineToHtml`、
    `list_serialization.dart`（48 行）。`markdown.dart` 少的 146 行是 T3.1 搬进 `paste_repair.dart` 的粘贴启发式。
    执行记录「删除 −7,460」的大头是差分基线 JSON 重置（`differential_baseline.json` −7,540），不是删产品代码，表述要改（见必须修改 3）。
  - ✅ 引擎调用点全部在边界上：复制（`selectionText` / `selectionHtml` / 单元格复制）、粘贴（`_parseOrNull`、`parseInlineMath`、HTML 粘贴里的 `mathRunSpans`）、
    AI（`editor_ai`、`main_shell` 导入）、整页 Markdown。渲染与输入热路径没有调用引擎。
  - ✅ 粘贴降级：`_parseOrNull` 捕获 `MarkdownCodecException` 后退回纯文本；复制失败时返回空串，`_copySelection` 退回只放纯文本。
  - ✖ **AI 读取当前页的降级不对**：`documentMarkdown()` 在引擎不可用时**静默返回空串**，`_openAiDialog` 的 `currentPage` 与 `readPageForEdits` 都拿它当页面正文。
    S3 之前这是纯 Dart、不会失败；现在 Web 上 wasm 没加载成功时，AI 会把有内容的页面当空页来回答、甚至据此生成编辑。这不在 02 §5.2 允许的降级里。
  - ✅ 引擎侧（审核方 Linux）：`cargo test --locked` `-p mica-md` 82 / `--features render` 81 / `--no-default-features` 81 / `-p mica-md-wasm` 10 / `-p mica-markdown` 185，全部 0 failed 0 ignored；
    clippy `-D warnings` 通过；`build-md-wasm.sh` 202,923 B（≤ 262,144）；正式 wasm 黄金重放 396 例逐字节。
  - ⚠ Flutter 侧（`flutter test` 2232 passed、`local-ci.sh all` 于 `dd183384`）**审核方未复跑**（本环境没有 Flutter），以执行方回执为准。
  - ✅ T3.7 分诊：B 1 / R 6 / H 3 / U 1，每条有结论；U-1（部分选中代码块复制带围栏）已由用户 2026-10-28 拍板。「用户可见变化」三条可进发版说明。
  - ✖ **T3.8 往返测试太弱**：35 个 fixture 里有 18 个因为「某一处已知损失」**整个文件跳过不比**（`markTestSkipped`）。例如 `13-links.md` 只因 title 会丢就整文件跳过，
    这个文件里链接地址、文字、其他标记出了新回归都抓不到。而这个测试自己写明是「Rust 写剪贴板 HTML → Dart 读回」这条跨语言链路的**唯一**防线。
  - ✅ 18 类损失都不是 S3 引入的：Rust Clipboard 写出的 HTML 与旧 Dart 在 30 个样本上逐字节相同（T3.4 移植测试），读回仍是同一个 `htmlToMarkdown`。
    但要注意：桌面和 Web 粘贴**都优先读 HTML**，编辑器没有内部剪贴板捷径，所以 Mica 内部复制粘贴时用户确实会碰到这些损失（链接 title、引用里的软换行、callout 类型等）。
- 契约问题裁决：
  - **B-1 `7ef6bbba` 改了冻结契约**（02 §3.2 原文「marks 若存在则按 §3.4 排序」规定的是输出顺序，不只是比较）。修复本身正确、影响面小：
    同区间两个标记时编辑器 `_normalize` 的顺序与原排序一致，只有三个及以上同区间且类型重复时有差别。**事后批准**，02 §3.2 / §3.4 已改为「线格式保留解析器产出顺序，比较时排序」。
    提醒：以后动冻结契约，先在 PR 里单列「契约变更」等审核方回复，不要先改后报。
- 必须修改（S3 内完成，G4 核对；**不阻塞 T3.6**——两条都不需要旧 Dart 实现做对照）：
  1. 引擎不可用时，`documentMarkdown()` 不能返回空串冒充空页：改为返回 `null`（或抛出），AI「读取当前页」与 `readPageForEdits` 遇到时明确报错、不发请求。
     配回归测试：`UnavailableMarkdownCodec` 下打开 AI 当前页 / AI 编辑，断言不会把空串当正文发出（先证明现状会失败）。
  2. `clipboard_mica_roundtrip_test` 改为**按损失项定点抹平再比**，不整文件跳过：只抹掉已知会丢的那一项（链接 / 图片 `title`、块内软换行折成空格、`marker`、`loose`、`alert`、
     代码 span 首尾空白），其余内容照比。只有确实无法定点抹平的（`33-tilde-fallback` 空代码块、`43-inline-nesting-budget`）可以保留整文件例外并写明理由。改完后例外清单应只剩这两个左右。
  3. 改正 S3 执行记录「删除行数」：分开写产品代码与基线 / fixture 的增删行数。
- 建议（不阻塞）：
  - Mica 自己复制出的 HTML 带 `data-mica-*` 标记，而同时放进剪贴板的 `text/plain` 是完整 Markdown。粘贴时若识别出是 Mica 自己的 HTML 就改读 Markdown，
    18 类损失在 Mica 内部复制粘贴时全部消失。这是产品行为变化，S3 之后由用户决定要不要做。
- 交给用户的决定：U-1 已决定；上面的「Mica 内部粘贴优先读 Markdown」是可选的后续改进。
- 结论：**G3 通过**，WP-D 可以执行 T3.6 删除 Dart 实现，继续 T3.9–T3.11；必须修改 1–3 在 S3 内完成，G4 核对。

## M2 复核 — 2026-10-27 — 通过

- 审核对象：`single-source` @ `e7b5cb10`。自 `9e604c43` 起 `crates/`、`clients/`、`Cargo.toml`、`Cargo.lock` 无改动，所以首轮 M2 的全量 Rust 门禁结果（1141 + 213 passed）仍然适用于本树。
- 核对结果（只核对首轮 M2 的 4 条必须修改）：
  1. ✅ `build-md-wasm.sh` 门槛 262144 B；`test-md-wasm-build.sh` 新增「205 KB 接受 / 300 KB 拒绝」两个边界用例。审核方在 Linux 上重建：
     `bash scripts/build-md-wasm.sh` exit 0，wasm 202,516 + JS 2,744 = **205,260 B**，wasm SHA `d7c21dd0…` 与首轮在 `9e604c43` 上的构建逐字节相同；
     执行方 Docker 构建记为 205,269 B（差 9 B，不影响门槛）。`test-md-wasm-build.sh`、`test-ci-paths.sh`、`check-file-size.sh`、`check-ignored-sources.sh` 全部 exit 0；
     正式产物黄金重放 396 例逐字节通过。
  2. ✅ Web 复制：真实门面 `serializeBlocks` + `renderClipboardHtml`，production dart2js、Chrome，1 万字符 / 298 块，三轮中位 4.6 / 4.3 / 4.1 ms，**≤ 16 ms**。
     审核方没有 Flutter 环境，此项以执行方回执为准；测量代码 `_measureCopy` 已提交，可在同一构建上重放。
  3. ✅ 归因已改正，并且执行方自查出另一处口径错误：serialize 的「Rust 0.63×」是拿 native 数字比 VM 下的 Dart，同轮 AOT 实为约 1.0× 持平。HTML 一项写明主因是新增的安全过滤。
  4. ✅ `b0064835` 删除 `scripts/md-wasm-evidence/`，S2 执行记录改为引用 `9e604c43:scripts/md-wasm-evidence/…`。
- 审核方顺手做的清理（本提交）：删除 `scripts/m2-linux-build.sh` / `m2-linux-run.sh`——与 `build-md-wasm.sh` 功能重复、写死容器路径 `/src` `/out`，仓库里没有调用方。
- 建议（不阻塞）：
  - 本轮 Web 复制与 Linux 构建的原始 JSON 只留在执行方本机工作树，没有进任何分支，别人无法取回。以后原始证据请提交到工作包分支并在记录里写提交号。
  - S2 执行记录「更正」一节末尾仍写着两项「作为待审核裁决项提交」，已被首轮 M2 裁决取代；节首「当前结论」表是对的，可不改。
- 结论：**M2 通过，S2 用 merge 提交并回 main。** 下一个停点是 G3（S3 的 T3.7 分诊、T3.8 往返测试完成后，T3.6 删除 Dart 代码之前）。

## M2 并回检查 — 2026-10-27 — 要求修改（4 条，修完只做快速复核）

- 审核对象：`single-source` @ `9e604c43`（S2 三个工作包 merge `2e5eb2e8` / `a5a5f1e2` / `7923e72a` 及其后的核心提交），S2 执行记录 P0 之后各节，README 验收快照。
- 核对结果：
  - ✅ Rust 门禁：Linux、新建空库 `mica_m2`、`MICA_REQUIRE_DB=1`，`bash scripts/local-ci.sh rust` exit 0，`receipt: 1141 passed (floor 500)`、store `213 passed (floor 1)`；
    另跑 `cargo test --locked -p mica-md --features render` exit 0。
  - ✅ 绑定只是转发：`clients/mica_flutter/rust/src/api/markdown.rs` 与 `crates/md-wasm/src/lib.rs` 都是十个一行转发，没有逻辑。
  - ✅ 消费方零改动：`crates/{app-core,api-server,interchange,mcp-server,mica-core,cli,infra}` 无 diff。`main.dart` 新增 `initMarkdownCodec` 一行，同 PR 压缩注释抵消，净 0。
  - ✅ 依赖：AGENTS.md 已登记 `wasm-bindgen`（仅 `crates/md-wasm`）与同号 CLI；`Swatinem/rust-cache` 是既有 action。
  - ✅ 构建链：`just dev-web` / `build-web` / `app chrome`、`ci.yml` 三处 Web 构建、`release.yml` 都先跑 `build-md-wasm.sh`；`index.html` 加载器在脚本缺失或实例化失败时落到 `micaMdError`，Flutter 可降级。
  - ✅ 正式 wasm 黄金重放（审核方 Linux 构建的产物）：`node scripts/test-md-wasm-gold.cjs` 9 个函数 396 例逐字节通过，engineInfo 指纹与 6 类错误后恢复通过。
  - ✅ 融合安全层 `492a9a42`：读了 `sanitize_html` 全文，各分支与「先 tagfilter 再 strip」的组合一一对应；分层实现保留为 `#[cfg(test)]` 参照，
    等价测试覆盖手选分支 + 2000 个生成用例。
  - ✅ 差分棘轮重建（T2.12）：通道从 `MicaDocument` 换成真实门面，旧维度 4336 → 3749，新增行内维度 1556 条单列，74 条是旧 CRDT 规范化掩盖的差异；逐条有审计记录，接受。
  - ✅ T3.4 Clipboard 提前到 S2：执行记录写明经用户授权；30 个旧 Dart 全选样本逐字节对上，危险 URL 过滤是刻意的安全差异。
  - ✖ **wasm 体积在 Linux 上超限**：审核方用 `scripts/build-md-wasm.sh` 在 Linux 构建，`9e604c43` 得 **205,260 B**（wasm 202,516 + JS 2,744），
    脚本拒绝；融合安全层之前的 `492a9a42^` 也是 205,241 B，与融合安全层无关。执行方在 Windows 上记的 204,107 B 没写测的是哪个提交与 SHA。
    CI 与发布镜像都在 Linux 上构建，按现状会直接失败。→ **用户决定上限改为 256 KiB**（02 §7 已改）。
  - ✖ 桌面复制两项超比值预算（serialize.10k 1.534×、html.10k 3.712×）→ **用户决定改为绝对预算**：复制 10k 选区（Markdown + HTML 合计）桌面、Web 各自 ≤ 16 ms（02 §7 已改）。
    桌面按执行方同轮 AOT 数据 2.238 + 2.656 ≈ **4.9 ms，达标**。**Web 的序列化与 HTML 没测过**（Web 只测了解析），缺这项证据。
  - ✖ 归因表述不准确：README 和执行记录说超预算「全部来自 FFI 边界」。HTML 一项不是——执行方自己的数据里 Rust Clipboard 渲染本体 1.542 ms 已高于 Dart 旧实现 0.715 ms，
    主要多在 Dart 旧实现从不做的安全过滤（标签过滤、危险属性 / URL 剥离）。审核方 Linux 探针：同量级 1 万字符 Clipboard wire 渲染 0.387 ms，其中 JSON 解码约 0.1 ms，引擎本身没有异常。
  - ✖ `scripts/md-wasm-evidence/` 有 22 个原始测量文件（约 5 万行，单个 JSON 3.7 万行），是证据不是源码，不应随 S2 进入 main 的树；仓库里没有任何脚本引用它。
  - ⚠ **审核方未复跑**（本环境没有 Flutter）：VM / Web / Windows 门面契约（42 + 11 + 11）、桌面 AOT 与 init 数字、浏览器解析比值，以执行方回执为准。
- 契约问题裁决：
  1. **Export HTML 跨端字节不同 → 接受执行方的候选方案**：跨端逐字节只约束 `clipboard` profile，`export` 随 `render` 特性变化（02 §1 / §3.1 已改）。
     理由：Dart 门面不暴露 Export，页面 HTML 导出在桌面走 `MicaDocument.exportHtml`、Web 走服务端，都不经过 wire，用户看不到这个差异。
     不给 wasm 开 `render`（体积），也不关桌面 `render`（会破坏现有本地导出）。
  2. **缩小 FFI 边界 → 现在不做**：按新的绝对预算桌面已达标，没有需要它的地方。若以后要做（例如把载体从 `String` 换成 UTF-8 字节），
     只要 §3 的 JSON 内容与 §5 门面不变，按「契约变更」单列在 PR 里报审核方即可，不需要改 wire 形状。
- 必须修改：
  1. `scripts/build-md-wasm.sh` 硬门槛改为 **262144 B**，`scripts/test-md-wasm-build.sh` 的边界用例同步；在 S2 执行记录写明：Linux 上正式产物的体积、对应提交号与 wasm SHA-256（以 Linux 为准，Windows 数字只作参考）。
  2. 补测 **Web 复制**：真实 `MarkdownCodec`、production dart2js、Chrome，S0 同一个 1 万字符选区，`serializeBlocks` + `renderClipboardHtml` 合计中位数 ≤ 16 ms；
     三轮，原始样本入档（证据放工作包分支，见第 4 条）。
  3. 改正 README 与 S2 执行记录里「超预算全部来自 FFI 边界」的说法：HTML 一项的主因是新增的安全过滤，并注明预算已按用户决定改为绝对值。
  4. 从 `single-source` 删除 `scripts/md-wasm-evidence/`（单独一个提交）；S2 执行记录里改为引用 `9e604c43` 这个提交的路径
     （`git show 9e604c43:scripts/md-wasm-evidence/<文件>` 仍可取回）。之后新的原始证据只留在工作包分支，执行记录写提交号。
- 建议（不阻塞）：
  - 同一提交 Linux 与 Windows 产物差约 1.1 KB，原因未查清（`--remap-path-prefix` 映射掉依赖路径只省 24 B，不是主因）。不影响新门槛，可以不追。
  - 执行记录已经很长（S2 一节 600+ 行），后续阶段请在节首给一张「当前结论」表，把历史纠错放到后面。
- 交给用户的决定：已决定两项（wasm 上限 256 KiB；复制改为 ≤ 16 ms 绝对预算），已写进 02 §7 与 05 D3 / D4。
- 结论：上面 4 条做完、在 04 §6 标「⏸ 待审核（M2 复核）」后，审核方核对这 4 条并在 Linux 上重新构建一次 wasm，通过即把 S2 并回 main。
  **M2 不停工**：S3 可以在 `single-source` 上继续推进到 G3。

## G1 复核 — 2026-10-27 — 通过

- 审核对象：`single-source` @ `f21daf18`（S0 修正 merge `5d8feb7`、S1 merge `f2bcb70` 之后只有文档提交）。
- 核对结果（只核对上一条 G1 记录的 4 条必须修改）：
  1. ✅ `20ecde0` 与 `6594f12` 逐行相同（`diff` 两者的 `crates/` 改动为空），只动 `crates/md-core` 三个文件；`git ls-tree single-source spike` 为空。
  2. ✅ `349f19a` 只改 `clients/mica_flutter/rust/Cargo.lock`，+10 行，与审核方上次在 Linux 上重新解析的结果一致；
     审核方在 `f21daf18` 上 `(cd clients/mica_flutter/rust && cargo check --locked)` exit 0。
  3. ✅ `88a46af` 删除了 ignored 的 `unmarked_cells_are_lossless_for_every_text`，`empty_marks_cells_document_the_current_loss` 的注释指向 02 §2.2 I3 除外项。
  4. ✅ 两个 merge 提交（先 S0 后 S1）；审核方在 `f21daf18` 上用 Linux、新建空库 `mica_g1r`、`MICA_REQUIRE_DB=1` 跑 `bash scripts/local-ci.sh rust`：
     exit 0，`receipt: 1106 passed (floor 500)`，`mica-core --features store` `receipt: 213 passed (floor 1)`；跑完工作区无改动（lock 未被重写）。
     执行方 Windows 回执是 1103 passed：差的 3 个是 `crates/cli` 里 `#[cfg(unix)]` 的测试（backup / config / export 各一），不是测试丢失。
  - 建议项 `contracts/*.json text eol=lf` 已随 `f576559` 提交。
- 必须修改：无。
- 结论：**G1 通过，02 §2 / §3 / §5 自本次起冻结**，之后改动须在 PR 里单列「契约变更」并经审核方同意。S0 + S1 用 merge 提交并回 main。
  WP-W 可以开始 T2.2；WP-D 可以开始 T2.5。

## G1 契约冻结 — 2026-10-27 — 要求修改（机械项，修完只做快速复核）

- 审核对象：S1 `worktree-agent-aaa10db22e269aaf4` @ `12c7ff2`；S0 初始 `c6446d5`（已在 single-source）+ 修正 `worktree-agent-acfa208a26ae27e64` @ `e122680`；
  各自的执行记录。
- 核对结果：
  - ✅ **全量 Rust 门禁（执行方的头号阻塞，已由审核方补跑）**：Linux、新建空库 `mica_g1`、`MICA_REQUIRE_DB=1`，
    在最终树 `12c7ff2` 上 `bash scripts/local-ci.sh rust` **exit 0**：structure / fmt / build / clippy `-D warnings` / migrations 全过，
    `receipt: 1106 passed (floor 500)`，`mica-core --features store` `receipt: 213 passed (floor 1)`。执行方在 Windows 上没跑通是环境问题，不是代码问题。
  - ✅ 消费方零改动：`git diff origin/main 12c7ff2 -- crates/{app-core,api-server,interchange,mcp-server,mica-core,cli,infra} clients scripts .github` 为空。
  - ✅ FFI 独立 lock：在 `clients/mica_flutter/rust` 重新解析后，`Cargo.lock` 只多 10 行（`mica-markdown` 改为依赖 `mica-md`，新增 `mica-md` 条目），
    没有任何 registry 包版本变化。
  - ✅ 公开面：`mica_md` 的 pub 项与 02 §2 一致，另多 7 个兼容入口（见裁决 #7）。
  - ✖→✅ 基准 ±5%：审核方用三份 release 二进制（`9af0de7` 基线 / `12c7ff2` S1 / `6594f12` S1+P0 优化）**同机交替各 15 轮**，取中位数：

    | 项 | 基线 ms | S1 | S1+`6594f12` |
    | --- | ---: | ---: | ---: |
    | import.10k | 0.499 | 0.528（**+5.8%**） | 0.521（+4.4%） |
    | import.100k | 2.499 | 2.575（+3.0%） | 2.566（+2.7%） |
    | export_markdown.10k | 0.275 | 0.257（−6.5%，变快） | 0.260（−5.5%，变快） |
    | 其余 6 项 | — | ±2% 内 | ±2% 内（87 字符档在计时精度以下） |

    import.10k 的四分位区间不重叠，**+5.8% 是真实回退**（约 0.03 ms / 1 万字符），来源是 T1.4 的 id 注入（临时 id + `assign_ids` 一遍替换）。
    并入 `6594f12` 后回到 +4.4%，在 ±5% 内；不要求再做别的优化。export_html.100k 的 −6.3% 在交替测量下不复现（−1.8%）。
  - ⚠ S0 修正 `e122680`：只读了提交与执行记录（往返输入改为 Rust 自身块、采样改为 7 次去 2 次、桌面探针只杀自己的子进程），逻辑正确；
    **审核方环境没有 Flutter，Dart 端回执未复跑**，以执行方回执（VM 2159 passed、原生指纹 + 对拍 5 passed）为准。
- 契约问题裁决（已写进 02 / S1 / S4 / S5，本提交）：
  1. `parse_body_json` 与 `parse_body`「经 serde 逐字节相同」按字面不成立 → **改为按 §3.2 投影后比较**（S1 T1.7 已改）。执行方现有的字段级测试符合新口径。
  2. 根块 type：解析器一直产出 `paragraph`，导入过的页面在库里就是这样存的（`crud.rs`、`import.rs` 都直接存 `import_markdown` 的结果）→
    **不改代码**，02 §6.1 写明「根块只按 `root_block_id` 识别，不看 type」。统一根块 type 不在本专项范围。
  3. `$$` 过滤放在 `parse_inline_math` 内、`math_run_spans` 不动 → **接受**，写进 02 §2。
  4. 空标记单元格的两类不无损（裸 URL / 邮箱读回多 `link`，首尾空白读回被去掉）→ **不采纳**「改存 `{text, marks: []}` 对象」的建议：
    那是存储行为变化，S1 的前提是行为不变；而且旧客户端读这种对象时也会把 text 当源码重新解析，得到的还是链接，对旧客户端没有好处。
    **I3 改为把这两类列为除外项**（02 §2.2、§6.4）。S4 之后编辑中和读回会不一致，这件事在 S4 T4.8 第 4 条交给用户选择。
  5. Sequential 与 root id 撞号时派生 `<prefix>_` → **接受**，写进 02 §2 `BlockIds`。
  6. FFI 独立 `Cargo.lock` → **允许**随路径依赖图更新（审核方已核对：只多 10 行，没有 registry 变化）。
  7. 核心追加的 7 个兼容入口 → **接受**；发布时怎么放，作为 S5 待用户决定的 Q6（推荐移进 `mica_md::compat`）。
- 必须修改（WP-R，在 S1 分支上做，做完合进 single-source）：
  1. 把 `6594f12`（`perf(md-core): T2.1 P0 remove quadratic parse_body …`）cherry-pick 到 S1 分支；只拿 `crates/md-core` 的改动，不带 `spike/`。
     跑 `cargo test -p mica-md`、`-p mica-md --no-default-features`、`-p mica-markdown`，回执贴执行记录。
  2. 提交 `clients/mica_flutter/rust/Cargo.lock` 的更新，附 `(cd clients/mica_flutter/rust && cargo check --locked)` 的通过回执；
     PR 里贴 `git diff --stat` 证明只动了这一个 lock 文件。
  3. 删掉 `crates/md-core/tests/cell_roundtrip.rs` 里 `#[ignore]` 的 `unmarked_cells_are_lossless_for_every_text`（契约已不再这样承诺），
     保留 `empty_marks_cells_document_the_current_loss`，在它的注释里指向 02 §2.2 I3 的除外项。
  4. 按 04 §4.1 用 merge 提交把 S0 修正（`e122680`）和 S1 分支依次合进 `single-source`；04 §6 进度表冲突时以合并后的实际状态为准。
     在合并后的 `single-source` 头上跑 `bash scripts/local-ci.sh rust`，回执贴 S1 执行记录。
- 建议（不阻塞）：
  - 执行方在 Windows 上被 `contracts/*.json` 检出成 CRLF 绊了一次，`.gitattributes` 里补一行 `contracts/*.json text eol=lf`（和 fixtures 那行同理）。可以随第 4 条一起提交。
  - 对拍记录数（4336）不是 bug 数，S3 分诊时按类别看即可。
- 交给用户的决定：无（第 4 条的产品选择在 S4 开始时再问）。
- 结论：上面 4 条做完、在 04 §6 标「⏸ 待审核（G1 复核）」后，审核方只核对这 4 条的回执，不重新审全部。复核通过后审核方
  把 02 顶部改为「已冻结」，并把 S0 + S1 并回 main。**在此之前 WP-R 只做这 4 条**；WP-D 可以继续准备 S3 的 T3.1（不依赖契约）。

## G2 wasm 判定 — 2026-10-27 — 通过（附条件）

- 审核对象：`ss/w/p0-costs` @ `68ae07f`（原 spike `d48ed4f`），S2 执行记录「P0」「G2 继续优化后的协调者复核」「独立 JSON 阶段计时复核」三节，
  实验分支 `spike/md-bench/summary-final.json` 等原始数据。用户已在 2026-10-27 选择「继续优化、预算不变」。
- 核对结果：
  - ✅ 解析性能：opt-3 + `6594f12` 下六组输入三轮比值中位数 0.62–0.79，**每轮都 ≤ 1.2×**（`summary-final.json` 的 `allRoundsPass` 全为 true），
    完整路径含 Rust JSON 编码、跨边界传字符串、Dart `jsonDecode` 和 `BlockSpec` 映射，没有缓存解析结果。
    对照组 opt-s 在 87 字符档是 1.165 / 1.128，勉强过线，所以选 opt-3 是对的。
  - ✅ 等价：每轮都比较了 wasm 与 Dart 的输出（`equivalence.equal: true`、FNV 一致），说明两条路径都真的跑了、结果相同。
  - ✅ 体积 123,073 B gzip（分文件计）≤ 200 KB；初始化 12.8 ms ≤ 60 ms；stable 工具链；不需要 COOP / COEP。
  - ✅ `6594f12` 代码审查：`parse_body` 的 O(n²) 查找改为一次索引 + 所有权移动；Sequential 去掉冗余去重集合（撞号已在前面处理）；
    wire 不再深拷贝、排序不再分配字符串。语义不变：root children 里的 id 由 `assign_ids` 保证唯一，按位置取块与原来按 id 找第一个等价。
    审核方的原生 A/B 也确认它不让任何基准变慢（见 G1）。
  - ⚠ 浏览器数据审核方没有复跑（本环境没有 Flutter），只核对了原始数据与汇总口径；只测了 Chrome。
- 必须修改（进入正式 S2 时做，不阻塞 T2.2 开始）：
  1. S2 T2.6 的 wasm release profile 改为 `opt-level = 3`（`lto = true`、`codegen-units = 1` 不变）；`build-md-wasm.sh` 的 200 KB gzip 硬门槛不变，
     按「wasm 与 JS 分别 `gzip -n -9` 后相加」的口径计。
  2. `6594f12` 按 G1 第 1 条进入核心，不从 spike 分支合并；`spike/` 目录不进 `single-source`。
  3. M2 时用正式 `crates/md-wasm`（十个 wire 函数，不含诊断导出）**重测**体积、初始化，以及 87 / 10k / 100k 三档的完整路径比值，结果贴 S2 执行记录。
     P0 的数字是诊断产物上测的，不能直接当 M2 证据。
- 建议（不阻塞）：M2 时在 Firefox 上抽测一次 100k 档，确认不是 Chrome 专属的结果。
- 交给用户的决定：无（「继续优化」已由用户决定；结果达标，不需要再走退出方案）。
- 结论：**wasm 方案成立**。WP-W 可以在 G1 复核通过、契约冻结后开始 T2.2（T2.2 依赖冻结的 wire 签名）；在那之前可以先做不依赖契约的 T2.6 构建脚本。
