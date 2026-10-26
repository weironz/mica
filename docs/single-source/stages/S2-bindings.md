# S2 绑定：桌面 FFI + Web wasm + Dart 门面（6–8 天）

## 目标

1. 桌面通过 FFI、Web 通过 wasm 调到**同一个** `mica_md::wire`，对同一输入输出逐字节相同。
2. Dart 侧有一个门面 `MarkdownCodec`（02 §5），两个实现，启动时初始化，失败可降级。
3. wasm 构建接进所有 Web 构建入口（本地、CI、发版、镜像），产物不提交、带指纹防旧。
4. 本阶段**不改任何编辑器行为**：门面建好、测好，但编辑器仍用 Dart 镜像（S3/S4 才切）。

## 前置

- **P0（T2.1）可以与 S1 并行**，用现有 `mica-markdown` 的 API 做一次性验证。
- 其余任务依赖 S1 契约冻结（`mica_md::wire` 存在）。

## 任务

### T2.1 P0 验证（1–2 天，验证代码不合并）

在单独分支上：

1. 临时 crate `spike/md-wasm-p0`（不加进 workspace），依赖 `mica-markdown`（`default-features = false`），用 wasm-bindgen 导出
   `parseBody(md)`（`import_markdown` → 根块 children → JSON）。S1 没合并前，`push_block` 还在用 `Uuid::new_v4()`：
   spike 里给 `uuid` 开 `js` feature 绕过（**只在 spike 里**；正式 crate 靠 S1 T1.4 去掉 uuid）。
2. `wasm-bindgen --target no-modules` 产物放进 `clients/mica_flutter/web/mica_md/`，按 02 §4.4 改 `index.html`，
   写一个最小的 `dart:js_interop` 调用（02 §5 的 web 实现雏形）。
3. **在真实的 Flutter Web 发布构建里**（`flutter build web --release`，nginx 或 `python3 -m http.server` 托管，Playwright 打开）测：

| 指标 | 阈值（事先定死，不达标即触发退出方案） |
| --- | --- |
| 体积（wasm + JS 胶水，gzip） | ≤ 200 KB |
| 实例化到 `micaMdReady` resolve | ≤ 60 ms |
| `parseBody` 87 字符 / 1 万字符 / 10 万字符（含 JSON 往返 + Dart `jsonDecode`） | ≤ S0 中同一浏览器里 Dart `markdownToBlocks` 的 1.2 倍 |
| 工具链 | stable Rust（`rust-toolchain.toml` 钉的版本）+ `wasm32-unknown-unknown` target，**无 nightly、无 `-Z build-std`** |
| COOP/COEP | 不需要（单线程 wasm-bindgen） |

4. 结果写进本文「执行记录 → P0」，然后**停点 G2**：进度表标「⏸ 待审核」，WP-W 停下等审核（[04 §4.2](../04-parallel-plan.md)）。
   **任何一项不达标**：审核方把「走退出方案 / 继续优化」的选择交给用户；退出方案见本文末「退出方案」。

### T2.2 依赖登记与工具链

- 选定 wasm-bindgen 版本：crates.io 上**发布满两周**的最新 `0.2.x`，记为 `=0.2.X`（精确钉死）。`wasm-bindgen-cli` 用同一个号。
- `AGENTS.md`「已批准的依赖及用途」加一行：

  ```
  - Web Markdown 引擎绑定：Rust `wasm-bindgen`（仅 `crates/md-wasm`）+ 构建工具 `wasm-bindgen-cli`（同号钉死，`scripts/build-md-wasm.sh` 校验）。
  ```

- `rust-toolchain.toml` 的 `[toolchain]` 加 `targets = ["wasm32-unknown-unknown"]`（rustup 读到会自动安装）。
  同步检查 `deploy/Dockerfile.api|.cli` 不受影响（它们不构建 wasm）。
- `scripts/release-check.sh` 的工具链检查段加：`wasm-bindgen --version` 与 `Cargo.lock` 里 `wasm-bindgen` 的版本一致，否则 REFUSED。

### T2.3 `crates/md-wasm`

按 02 §4.2 建 crate，导出 §3.3 全部函数（`js_name` 用 camelCase）。另外：

- `crates/md-wasm/build.rs`：计算 `crates/md-core/{Cargo.toml,src}`、`crates/md-wasm/{Cargo.toml,src,build.rs}` 的源码指纹
  （算法同 `clients/mica_flutter/rust/build.rs`：FNV-1a 64，按 `/` 相对路径排序，`path \0 contents \0`），
  通过 `cargo:rustc-env=MICA_MD_SOURCE_HASH` 注入，`engineInfo()` 返回它。
- 根 `Cargo.toml` 加：

  ```toml
  [profile.release.package.mica-md-wasm]
  opt-level = "z"
  codegen-units = 1
  ```

  （`lto` 只能在 profile 级设置，不能按 package 设；如需 LTO，用 `CARGO_PROFILE_RELEASE_LTO=true` 只在 `build-md-wasm.sh` 里开，不影响服务端构建。）
- 测试：`crates/md-wasm/tests/forward.rs`（宿主机上跑）：每个导出函数的结果与直接调用 `mica_md::wire::*` 相同。

### T2.4 桌面 FFI

按 02 §4.1：

1. `clients/mica_flutter/rust/src/api/markdown.rs`（十个 `#[frb(sync)]` 转发函数）、`api/mod.rs` 加 `pub mod markdown;`、
   `rust/Cargo.toml` 加 `mica-md` 路径依赖。
2. `rust/build.rs` 的 `ROOTS` 加 `crates/md-core/Cargo.toml`、`crates/md-core/src`（**必须**，否则改了核心而库没重编时指纹查不出来）。
3. 在 `clients/mica_flutter` 运行 `flutter_rust_bridge_codegen generate`（2.12.0）。提交生成物。
   生成后 `git diff --stat` 应只多出 `lib/src/rust/api/markdown.dart` 和 `frb_generated*` 的改动；其他文件有改动就停下查原因。
4. `cargo build --release --manifest-path clients/mica_flutter/rust/Cargo.toml` 后，`flutter test test/cloud_sync_seed_healthy_test.dart`
   仍通过（指纹机制没坏）。

### T2.5 Dart 门面

新目录 `clients/mica_flutter/lib/markdown/`：

```
mark.dart                 class Mark（从 editor/marks.dart 原样剪切过来；marks.dart 改为 `export '../markdown/mark.dart';`）
markdown_codec.dart       02 §5 的接口、typedef、异常、访问器、initMarkdownCodec、debugSetMarkdownCodec、UnavailableMarkdownCodec
markdown_wire.dart        纯 Dart：BlockSpec/InlineText/Mark ⇄ JSON
markdown_codec_io.dart    Future<MarkdownCodec> createMarkdownCodec()：await initNativeRust()（幂等，吞掉「已初始化」）→ _FfiMarkdownCodec
markdown_codec_web.dart   Future<MarkdownCodec> createMarkdownCodec()：await micaMdReady（10 s 超时）→ _WasmMarkdownCodec
```

- `BlockSpec` typedef 从 `editor/markdown.dart` **移到** `markdown_codec.dart`；`markdown.dart` 改为
  `export '../markdown/markdown_codec.dart' show BlockSpec;`（S3 删 `markdown.dart` 时再改两个使用者的 import）。
- `_FfiMarkdownCodec` 每个方法：`markdown_wire` 编码 → 调 frb 生成的 `mdParseBody(...)` 等 → 解码。frb 抛出的异常包成 `MarkdownCodecException`。
- `_WasmMarkdownCodec`：

```dart
// markdown_codec_web.dart（节选）
import 'dart:js_interop';

@JS('micaMd') external _MicaMd? get _micaMd;
@JS('micaMdReady') external JSPromise<JSAny?>? get _micaMdReady;
@JS('micaMdError') external JSString? get _micaMdError;

extension type _MicaMd._(JSObject _) implements JSObject {
  external String parseBody(String markdown, bool fragment);
  external String parseInlineBatch(String sourcesJson);
  external String decodeCells(String cellsJson);
  external String encodeCells(String inlinesJson);
  external String serializeBody(String blocksJson, String assetsJson);
  external String serializeInline(String inlineJson);
  external String renderHtmlBody(String blocksJson, String imageSrcsJson, String profile);
  external String mathRunSpans(String source);
  external String parseInlineMath(String source);
  external String engineInfo();
}

Future<MarkdownCodec> createMarkdownCodec() async {
  final ready = _micaMdReady;
  if (ready == null) throw const MarkdownCodecException('mica_md.js was not loaded (index.html)');
  await ready.toDart.timeout(const Duration(seconds: 10));
  final md = _micaMd;
  if (md == null) throw MarkdownCodecException(_micaMdError?.toDart ?? 'wasm init failed');
  return _WasmMarkdownCodec(md);
}
```

  wasm 函数抛出的 JS 异常在 Dart 里是 `JSObject`：用 `on Object catch (e)` 包成 `MarkdownCodecException('$e')`。
- `initMarkdownCodec()`（`markdown_codec.dart`）：

```dart
Future<void> initMarkdownCodec() async {
  final started = DateTime.now();
  try {
    _codec = await platform.createMarkdownCodec();
  } catch (error, stack) {
    logCrash('markdown codec unavailable: $error\n$stack');   // diagnostics 的 crash log（不受「诊断」开关控制）
    _codec = const UnavailableMarkdownCodec();
  }
  debugPrint('markdown codec ready in ${DateTime.now().difference(started).inMilliseconds} ms');
}
```

### T2.6 构建脚本 `scripts/build-md-wasm.sh`

```bash
#!/usr/bin/env bash
# Build the web Markdown engine (crates/md-wasm) into the Flutter web folder.
#   bash scripts/build-md-wasm.sh            # release build, as shipped
# Output is generated, never committed (.gitignore): a committed bundle goes
# stale silently — the same failure the FFI source fingerprint exists to stop.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

want=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"/\1/p' | head -1)
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
[ "$have" = "$want" ] || fail "wasm-bindgen-cli is '${have:-missing}', Cargo.lock pins $want — cargo install wasm-bindgen-cli --version $want --locked"

CARGO_PROFILE_RELEASE_LTO=true cargo build -p mica-md-wasm --target wasm32-unknown-unknown --release --locked
out=clients/mica_flutter/web/mica_md
rm -rf "$out" && mkdir -p "$out"
wasm-bindgen --target no-modules --no-typescript --out-dir "$out" --out-name mica_md \
  target/wasm32-unknown-unknown/release/mica_md_wasm.wasm

budget_kb=${MICA_MD_WASM_BUDGET_KB:-200}
size_kb=$(( $(cat "$out/mica_md_bg.wasm" "$out/mica_md.js" | gzip -9 | wc -c) / 1024 ))
[ "$size_kb" -le "$budget_kb" ] || fail "mica_md is ${size_kb} KB gzip, budget ${budget_kb} KB (02 §7)"

# 指纹：由 crates/md-wasm/build.rs 注入二进制；这里另存一份明文给 VM 测试比对（T2.10）
cargo run -q -p mica-md-wasm --example print_fingerprint > "$out/SOURCE_FINGERPRINT"
rm -rf clients/mica_flutter/test/support/mica_md && cp -r "$out" clients/mica_flutter/test/support/mica_md
note "mica_md: ${size_kb} KB gzip → $out"
```

（`print_fingerprint` 是 `crates/md-wasm/examples/print_fingerprint.rs`，打印同一个 `MICA_MD_SOURCE_HASH`。）

### T2.7 Web 加载

- `web/index.html` 按 02 §4.4 加两段 script（`wasm_bindgen(...)` 的参数形式以 T2.2 选定版本的官方文档为准，执行记录里写明）。
- 浏览器测试的加载方式仿照 `test/web_doc_mirror_test_web.dart` 注入 yjs 的做法：测试启动时动态插入 `<script src="support/mica_md/mica_md.js">`，
  然后执行与 index.html 相同的初始化，等待 `micaMdReady`。

### T2.8 启动接线

- `lib/main.dart` `main()`：在 `registerYjsSelfTest();` 附近加 `await initMarkdownCodec();`。
- `main.dart` 库在行数棘轮上：**本阶段**必须在同库其他地方净减一行（例如合并一处单语句 `setState`，第 3 条做过同类处理），
  `bash scripts/check-file-size.sh` 通过。不要动预算表的数字（只能往下收紧）。
- 测量：桌面发布版启动 5 次，记录 `markdown codec ready in N ms` 的中位数；Web 用 Playwright 量首帧时间，与 S0 T0.4 对比。

### T2.9 契约测试（两个平台跑同一套断言）

1. Rust 侧生成黄金文件：`crates/md-core/tests/wire_gold.rs`，`GEN_GOLD=1 cargo test -p mica-md --test wire_gold` 时把
   「conformance fixtures 的 `.md` → `parse_body_json`」「对应块 → `serialize_body_json` / `render_html_body_json`」「若干行内样例 →
   `parse_inline_batch_json` / `serialize_inline_json` / `decode_cells_json` / `encode_cells_json` / `math_run_spans_json` / `parse_inline_math_json`」
   的输出写到 `crates/md-core/tests/fixtures/wire/*.json`；平时运行时断言当前输出与黄金文件相同。
2. Dart：
   - `test/markdown_codec_contract_body.dart`：`void runCodecContract(Future<MarkdownCodec> Function() make)`，读黄金文件，逐项断言门面输出
     （解码后的结构）与黄金文件相同。
   - `test/markdown_codec_contract_test.dart`：VM 入口，`make = () async { await useNativeMarkdownCodec(); return markdownCodec; }`。
   - `test/markdown_codec_contract_web_test.dart`：`@TestOn('browser')`，`make` = T2.7 的注入加载。会被 `scripts/browser-tests.sh` 自动发现。
   - 浏览器里读不到文件系统：黄金文件由 `build-md-wasm.sh` 一并复制到 `test/support/mica_md/gold/`，测试用 `fetch` 读取。
3. 两个入口都必须有**通过数回执**（`flutter_receipt`）。

### T2.10 防旧产物

- `test/support/native_build.dart` 的 `sourceFingerprint(repo, roots)` 已是通用函数。新增 `test/web_engine_fresh_test.dart`（VM）：
  若 `web/mica_md/SOURCE_FINGERPRINT` 存在，用 `crates/md-core`、`crates/md-wasm` 的 ROOTS 重算并比对，不同则失败并提示
  `bash scripts/build-md-wasm.sh`；文件不存在则 `markTestSkipped`（纯 Rust/VM 开发不强制构建 wasm）。
  **门禁里**（local-ci flutter 部分、CI flutter 作业）先构建 wasm，所以门禁里这个测试一定会真正比对。
- 桌面侧：`initCheckedNativeRust` 已覆盖（T2.4 第 2 步把 md-core 加进 ROOTS）。

### T2.11 构建与 CI 接入

按 03 §4 的表逐项接上。额外：

- `scripts/local-ci.sh` flutter 部分：在 `flutter build web` 和 `browser-tests.sh` 之前 `bash scripts/build-md-wasm.sh`；
  Windows 主机（`host_os == windows`）上同样构建（wasm 构建在 Windows 可行），浏览器套件仍按现状只在 Linux/macOS 跑。
- CI：需要 `wasm-bindgen-cli` 的作业加一步 `cargo install wasm-bindgen-cli --version <钉的号> --locked`（用 rust-cache 缓存 `~/.cargo/bin`）。
- `scripts/test-ci-paths.sh` 增加用例：只改 `crates/md-core/src/x.rs` 时，rust、flutter、web 作业都被选中。

### T2.12 差分工具切到门面

S0 T0.2 的 Rust 侧从 `MicaDocument` 路径改为 `markdownCodec.parseBlocks` / `serializeBlocks`，并新增行内对比
（Dart `parseInline` vs `markdownCodec.parseInline`、Dart `inlineToMarkdown` vs `serializeInline`）。
重新生成 `differential_baseline.json`：`crdt-normalization` 类应清零，其余类别数量变化写进执行记录（预期只减不增）。

### T2.13 性能验收

按 02 §7 逐项测（VM/FFI、Chrome/wasm 各一套），与 S0 基线对比，表格写进执行记录。超预算按 02 §7 末尾的规则处理。

### T2.14 Windows 桌面验证

- `integration_test/markdown_codec_test.dart`：`setUpAll(initCheckedNativeRust)`，调用 `runCodecContract`（同一份断言）。
- 在 Windows 上：`flutter test integration_test/markdown_codec_test.dart -d windows` 通过；安装包冒烟（`just` 的 Windows 构建）后，
  应用启动日志里有 `markdown codec ready in N ms`。截图或日志片段贴进执行记录。

## 验收（= 并回检查 M2 的材料；M2 不停工，但 S2 并回 main 要等它通过）

- [ ] P0 全部阈值达标（或已按退出方案处理并经审核方同意）。
- [ ] `cargo test -p mica-md --test wire_gold`、`cargo test -p mica-md-wasm` 通过。
- [ ] VM 契约测试、浏览器契约测试、Windows 集成契约测试三处都通过，回执齐全。
- [ ] 桌面与 Web 对全部黄金输入逐字节一致（契约测试即证明）。
- [ ] 所有 Web 构建入口都会先构建 wasm；`git status` 干净（产物被忽略）；`check-ignored-sources.sh` 通过。
- [ ] 启动耗时、性能数字在预算内。
- [ ] 编辑器行为**没有任何变化**：S0 的差分棘轮、全部 Flutter 测试照常通过。
- [ ] `main.dart` 库行数没有增长。

## 退出方案（P0 不达标时）

不删 Web 端的 Dart 镜像，改为「桌面 + 服务端单一实现，Web 保留镜像 + 差分棘轮守护」：

- `markdown_codec_web.dart` 的实现改为调用现有 Dart 镜像（`markdownToBlocks` 等），**镜像代码挪进 `lib/markdown/web_mirror/`，
  只被 web 实现引用**；桌面完全不再触碰它。
- S3、S4 照常进行（桌面走 FFI），但 S3/S4 的「删除镜像」步骤改为「把镜像收拢到 `web_mirror/`」。
- 差分棘轮（S0 T0.2）改为 CI 必跑，镜像的任何偏离立刻失败。
- 记录不达标的数字和原因，等 wasm 工具链或引擎体积有变化时重新评估。

## 执行记录

（执行方追加。P0 结果放在最前面。）

### P0 成本归因与候选优化（T2.1，2026-10-27，复核收尾）

**结论：推荐 opt-level=3。** 修正采样与汇总方法后，opt-3 在原 P0 三档及三个代表性输入上，每档三轮全部满足完整路径 ≤同页 Dart×1.2；体积和初始化也满足硬预算。G2 仍待正式审核，G1 未裁决，未开始 T2.2+；本轮无新增 core 优化。

**分支/提交**：`ss/w/p0-costs` 基于 S1 候选 `12c7ff2`；核心候选单独提交 `6594f12`，上一轮实验 `8a2af3f`。只修改 spike 与本执行记录；未 merge/push。

#### 方法、输入与纠错

- 真实 Flutter 3.47.1 / Dart 3.13.1 release，临时入口启动真实 app.main()；同页 Chrome 156 对照生产 markdownToBlocks。Rust 1.98.0 stable、wasm32-unknown-unknown、wasm-bindgen crate/CLI 0.2.129，无 nightly/-Z build-std。
- 最终 full/Dart 采样采用 **AB/BA 交替的 15 组批次**，预热后校准批量；同页连续三轮。run_paired.cjs 为 opt-3 全六输入，run_short.cjs 为 opt-s 两个短输入。orderedSamples 保留原采样顺序。旧 run_stages 是整阶段固定顺序，旧报告声称交替不实，现纠正；旧原始数据保留。
- 汇总比值为 median(run1 full/Dart, run2 full/Dart, run3 full/Dart)。aggregate.cjs 原先错误地计算“阶段中位数之比”，已重算全部历史 summary-costs.json，保留 perRunRatios 及 legacyRatioOfStageMedians；旧 opt-3 原 P0 比值 0.736/0.451/0.514 作废，更正为 0.782/0.447/0.531（仍是固定顺序历史数据，最终以本节 AB/BA 数据为准）。
- 原 P0 输入保持 87/10,000/100,000 字符原字节，spike/inputs/p0_*.md 为证；s0_87 与 S0 hotPath87 原字节一致。s0_10k 实际 **9,997** 字符、s0_100k **99,972** 字符，按行边界截断；S0 使用精确 substring 截断，故两者仅为**近似同形代表性复核**，不是 S0 官方基线或完全同输入。
- 性能样本外比较真实规范化块与 marks：丢弃 id/children、对象键排序、marks 按(start,end,type)排序。最终全部样本 differingBlocks=0、blockCountDelta=0；初轮基线对象键次序误报保留在原数据并明确作废，不修改原数据。尚不能外推其他语义输入。
- 服务器由 preview_start(md-p0-static) 启动，无 COOP/COEP、Cache-Control:no-store；crossOriginIsolated=false。

#### 最终完整路径：每轮实测（ms）

| 配置 | 输入/实际字符 | 轮次 | full ms | Dart ms | 同轮比值 |
| --- | --- | ---: | ---: | ---: | ---: |
| opt-3 | p0_87 / 87 | 1 | 0.012012 | 0.013770 | 0.872 |
| opt-3 | p0_87 / 87 | 2 | 0.011914 | 0.015137 | 0.787 |
| opt-3 | p0_87 / 87 | 3 | 0.011572 | 0.014746 | 0.785 |
| opt-3 | p0_10k / 10000 | 1 | 1.343750 | 2.112500 | 0.636 |
| opt-3 | p0_10k / 10000 | 2 | 1.337500 | 2.075000 | 0.645 |
| opt-3 | p0_10k / 10000 | 3 | 1.318750 | 2.025000 | 0.651 |
| opt-3 | p0_100k / 100000 | 1 | 12.750000 | 18.200000 | 0.701 |
| opt-3 | p0_100k / 100000 | 2 | 12.750000 | 20.700000 | 0.616 |
| opt-3 | p0_100k / 100000 | 3 | 12.500000 | 24.900000 | 0.502 |
| opt-3 | s0_87 / 87 | 1 | 0.011035 | 0.012695 | 0.869 |
| opt-3 | s0_87 / 87 | 2 | 0.010889 | 0.014160 | 0.769 |
| opt-3 | s0_87 / 87 | 3 | 0.010840 | 0.014063 | 0.771 |
| opt-3 | s0_10k / 9997 | 1 | 2.825000 | 4.200000 | 0.673 |
| opt-3 | s0_10k / 9997 | 2 | 2.775000 | 4.050000 | 0.685 |
| opt-3 | s0_10k / 9997 | 3 | 2.812500 | 4.025000 | 0.699 |
| opt-3 | s0_100k / 99972 | 1 | 32.800000 | 50.900000 | 0.644 |
| opt-3 | s0_100k / 99972 | 2 | 32.300000 | 49.200000 | 0.657 |
| opt-3 | s0_100k / 99972 | 3 | 32.300000 | 49.600000 | 0.651 |
| opt-s | p0_87 / 87 | 1 | 0.015137 | 0.013184 | 1.148 |
| opt-s | p0_87 / 87 | 2 | 0.014844 | 0.012744 | 1.165 |
| opt-s | p0_87 / 87 | 3 | 0.014746 | 0.012598 | 1.171 |
| opt-s | s0_87 / 87 | 1 | 0.013770 | 0.012207 | 1.128 |
| opt-s | s0_87 / 87 | 2 | 0.013818 | 0.012207 | 1.132 |
| opt-s | s0_87 / 87 | 3 | 0.013770 | 0.012207 | 1.128 |

| 配置 | 输入 | 三轮同轮比值中位数 | 每轮均 ≤1.2 |
| --- | --- | ---: | --- |
| opt-3 | p0_87 | 0.787 | 通过 |
| opt-3 | p0_10k | 0.645 | 通过 |
| opt-3 | p0_100k | 0.616 | 通过 |
| opt-3 | s0_87 | 0.771 | 通过 |
| opt-3 | s0_10k | 0.685 | 通过 |
| opt-3 | s0_100k | 0.651 | 通过 |
| opt-s | p0_87 | 1.165 | 通过 |
| opt-s | s0_87 | 1.128 | 通过 |

opt-s 短输入的前一次固定窗口补测出现 p0_87=1.204819 的单轮，原记录 short-opts.txt 保留；最终 AB/BA 数字如上。推荐 opt-3 的延迟余量，不再要求用户在 s/3 间重新选择。

#### 历史配置比较（纠正为同轮比值中位数，固定窗口采样）

| 配置 | p0_87 | p0_10k | p0_100k | s0_87 | s0_10k（近似） | s0_100k（近似） |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| base-z | 1.757 | 1.124 | 1.629 | 1.726 | 0.982 | 1.163 |
| opt-z | 1.263 | 0.794 | 0.851 | 1.354 | 0.847 | 0.829 |
| opt-s | 1.030 | 0.581 | 0.593 | 未测 | 0.580 | 0.624 |
| opt-3 | 0.782 | 0.447 | 0.531 | 未测 | 0.535 | 0.545 |

#### 阶段成本与单位修正（历史独立探针，非可加和账本）

- base-z 的 p0_100k：parseOnly **37.2 ms**；parseBody **44.0 ms**；echoJson **1.56875 ms（不是 µs）**；Dart jsonDecode **1.3375 ms**；BlockSpec 映射 **0.659375 ms**；full **45.8 ms**。87 字符 echoMd=0.000756835938 ms=**0.756835938 µs**。空循环约0.00002 ms，noop约0.000012 ms。
- opt-z p0_100k：parseOnly 20.1 ms、parseBody 23.5 ms、full 25.8 ms；历史 opt-3 parseOnly 10.95 ms、parseBody 12.7 ms、full 15.1 ms。opt-3 没有独立测 echo/decode/map，旧表沿用 opt-z 值至 opt-3 的列现撤销，不当实测。
- **Stage 3 尚无孤立测量。** parseBody−parseOnly 仅是粗估：parseOnly 含结果 digest 并返回 f64，而 parseBody 包含 BlockJson 映射/排序/JSON 编码及 JSON 字符串返回，两者并不同构。差值不可称精确编码耗时，禁止与 echo/decode/map 相加。分配/GC/热身使各阶段不可加和。

#### 匹配 UUID/Sequential（方向 Sequential → UUID）

旧 +48% 使用了 parse_body（含索引/移动）对 parse（含root digest）的不对称探针，**纯 UUID 归因作废**，旧 uuid-run*.txt 原样保留。新 parseIdDigest 两边使用同一 parse、Document mode、root="root"、同一含 root digest，仅 BlockIds::Sequential{prefix:"b"} 与 Uuid 不同；AB/BA 三轮。差值是完整 ID 策略（含唯一性 bookkeeping）成本，仍不是随机数系统调用的单独成本。

| 输入 | 轮次 | Sequential ms | UUID ms |
| --- | ---: | ---: | ---: |
| p0_87 | 1 | 0.013965 | 0.017090 |
| p0_87 | 2 | 0.013428 | 0.016406 |
| p0_87 | 3 | 0.013037 | 0.015820 |
| p0_10k | 1 | 1.725000 | 2.487500 |
| p0_10k | 2 | 1.706250 | 2.462500 |
| p0_10k | 3 | 1.706250 | 2.462500 |
| p0_100k | 1 | 16.300000 | 23.900000 |
| p0_100k | 2 | 16.400000 | 23.900000 |
| p0_100k | 3 | 16.400000 | 24.100000 |

#### 体积与初始化（最终可复算口径）

measure_sizes.cjs 对保留的每个实际产物执行 **GNU gzip 1.14 -n -9 -c**，按 wasm/JS **分别压缩后相加**作为分文件传输总量；sizes-costs.json 记录各文件 SHA-256、原始 bytes、Node zlib 历史值及拼接压缩值。下面表格由该 JSON 自动生成：

| 配置 | wasm gzip B | JS gzip B | 分文件传输合计 B | 历史拼接 gzip B（GNU） | init 中位 ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| base-z | 99157 | 2535 | 101692 | 101624 | 12.7 |
| opt-z | 99353 | 2535 | 101888 | 101824 | 12.3 |
| opt-s | 108345 | 2535 | 110880 | 110869 | 13.6 |
| opt-3 | 120538 | 2535 | 123073 | 122979 | 12.8 |
| opt-z-uuid | 101372 | 2800 | 104172 | 104174 | 未测 |

旧表合计 102184/102346/111224/123226/104811 是 **Node 24.12.0 zlib 的拼接压缩**，而单列是各文件 Node 压缩大小，故不能相加对应。旧 Node 的 opt-3 分文件总量为 123260 B，旧拼接值 123226 B；本次 GNU 分文件总量 **123073 B**（120.188 KiB），超过120×1024软目标 **193 B**，仍远低于200×1024硬预算。压缩器实现差异也已分开，未宣称是产物版本变化。
所有体积都含 echo/noop/parseOnlyDigest 等诊断导出，为**本 spike 的体积上界**，不是正式 §3.3 十API产物体积；正式 S2 必须重新验收。新 ID 探针只在 matched-z-uuid 产物中，最终性能复测 opt-3/opt-s 使用原保留产物，served SHA 与 sizes JSON 校验一致。
核心源码 fingerprint=6b193da84a7b630a 对应6594f12，字节哈希在summary-final.json；无源码/wasm不匹配。

#### 回执、范围与待审核

- 本轮新增/修正仅 spike 与本执行记录，核心6594f12未再改动。final host receipt **4 passed（floor 4）**：7输入原Rust wire JSON SHA-256黄金、marks排序、UUID/Sequential块等价、匹配探针消费值等价。shellcheck、just check-structure、check-ignored-sources通过（receipt-final.txt）。此前核心55 passed/1 ignored、facade185 passed保留，非本轮重复跑大套件。
- **推荐 opt-3：硬预算全部达标，G2待审核正式判定；G1仍阻塞，T2.2未启动。** 不问用户重复选配置，不合并/推送/发版。Firefox/Safari、桌面FFI及正式十API wasm体积未验证；stage3独立绝对成本仍未验证，粗估仅供理解。
- 本轮raw数据：final-opt3-paired.txt、final-opts-short-paired.txt、final-ids-paired.txt；summary-final.json与sizes-costs.json为最终汇总。旧数据不删除，不回写观测值。
