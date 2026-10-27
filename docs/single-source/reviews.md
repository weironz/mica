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
