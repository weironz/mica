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
