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

> **证据归档说明（M2 复核后）**：本记录原先引用的原始证据目录 `scripts/md-wasm-evidence/` 已按审核要求从工作树删除。
> 文中以 `9e604c43:scripts/md-wasm-evidence/<文件>` 形式标注的路径，可在该提交中取回：
> `git show 9e604c43:scripts/md-wasm-evidence/<文件>`（或 `git checkout 9e604c43 -- scripts/md-wasm-evidence/`）。
> 其余结果均可在集成分支 `single-source` 的对应提交与门禁日志中复现。

### 当前结论（2026-10-27，M2 复核后）

| 项 | 结论 | 依据 |
| --- | --- | --- |
| 契约 | 冻结；跨端逐字节只约束 Clipboard，Export HTML 随 `render` 特性变化——M2 裁决接受 | [02 §3.1](../02-architecture.md)、[reviews.md](../reviews.md) |
| 预算 | wasm ≤ 262,144 B（以 Linux 构建为准）；复制 10k ≤ 16 ms 绝对值；解析 ≤ Dart ×1.2 | [02 §7](../02-architecture.md)、[05 D3/D4](../05-review-checklist.md) |
| wasm 体积 | Linux 205,269 B（审核方同树独立构建 205,260 B） | 本文「M2 复核实测」 |
| Web 解析 / 复制 | 三档比值 1.073 / 0.523 / 0.569；复制三轮中位 4.6 / 4.3 / 4.1 ms | 本文 T2.13、M2 复核实测 |
| 桌面 | init 2 ms、解析 0.873 / 0.640 / 0.616、复制约 4.9 ms、边界 0.0124 ms、VM 契约 42/0/0 | 本文「最终桌面验收」 |
| 状态 | 审核要求的 4 条修改已完成，等快速复核 | [04 §6](../04-parallel-plan.md) |

下文按时间保留历史记录；出现矛盾时以本表、[reviews.md](../reviews.md) 与 02 为准。

### P0（T2.1，2026-10-26，WP-W）

以下为单次 P0 验证结果记录；本次验证代码只在本分支 `worktree-agent-a7950b28e1e2a2c23`，不进入 main / `single-source`。对照基线说明：S0 基线尚未产出，因此性能比较采用本次同一浏览器、同一页面内实测的现有 Dart `markdownToBlocks`。工具链与运行环境：Rust 1.98.0 stable（`rust-toolchain.toml`）+ `wasm32-unknown-unknown`；wasm-bindgen crate 与 CLI 均为 0.2.129；Flutter 3.47.1 / Dart 3.13.1，真实 `flutter build web --release --no-web-resources-cdn`；本地静态服务器 + Playwright，HeadlessChrome/156.0.0.0（Windows 11 Pro）。

方法：
1. `spike/md-wasm-p0`（非 workspace 成员）依赖现有 `crates/markdown`，`default-features = false`，导出 `parseBody(md)` → `import_markdown` → 根块 children → `BlockJson` JSON 字符串。构建参数约等于 T2.6 的目标：`opt-level = "z"`、`codegen-units = 1`、`lto = true`。
2. `wasm-bindgen --target no-modules` 产物拷贝到 `clients/mica_flutter/web/mica_md/`（已加 `.gitignore`），`web/index.html` 如约加入两段脚本，`micaMdReady` resolve 前 record 初始化成本。
3. `spike/md-bench/main.dart` 仅作为临时 Dart 入口：启动真实 `app.main()`，并在首次完整引擎输出就绪后提供 `runMdP0()`。`runMdP0` 中跑最深等价度的 full-path 对比：wasm 路径为 Markdown → wasm 字符串返回 → `json.decode` → 构建 `BlockSpec`；Dart 路径为 `markdownToBlocks`；同一文档、同一浏览器、同批次交替顺序测 15 个样本。
4. 服务器返回头：`Cache-Control: no-store`，无 COOP / COEP；跨域隔离结果如实记录。

**初步和最终结论：体积 / 初始化 / 工具链 / COOP-COEP 达标，解析性能三项（87 字符 / 1 万 / 10 万字符）均超过 1.2 倍门槛。** 建议 P0 判定为**不达标**（性能项），并由审核方决定继续优化或进入退出方案。

#### P0 阈值逐项结果

| 指标 | 阈值 | 实测 | 结果 |
| --- | --- | ---: | --- |
| wasm + JS 胶水 gzip | ≤ 200 KB | **95,412 B（93.18 KiB，分别 gzip -9 后相加：wasm 92,997 B + JS 2,415 B）**；拼接后 gzip 为 95,498 B | **通过** |
| 实例化到 `micaMdReady` resolve | ≤ 60 ms | 5 个新上下文：20.60 / **14.70** / 14.40 / 15.00 / 16.30 ms，中位数 **15.00 ms** | **通过** |
| 87 字符解析（含 JSON 往返 + Dart `jsonDecode`） | ≤ 同浏览器 Dart `markdownToBlocks` × 1.2 | 三轮比值 **2.11× / 1.75× / 1.89×** | **不通过** |
| 1 万字符解析（同上） | ≤ ×1.2 | 三轮比值 **1.37× / 1.35× / 1.50×** | **不通过** |
| 10 万字符解析（同上） | ≤ ×1.2 | 三轮比值 **1.57× / 1.44× / 1.52×** | **不通过** |
| 工具链 | stable Rust + wasm32 target，无 nightly / 无 `-Z build-std` | stable 1.98.0，非 nightly，未用 `-Z build-std` | **通过** |
| COOP / COEP | 不需要 | 实测服务器仅 `Cache-Control: no-store` + 无 COOP/COEP，`crossOriginIsolated` 为 false，wasm 正常运行 | **通过（不需要）** |

#### 关键测量数据（供复算）

- 体积（gzip -9）：wasm 92,997 B、`mica_md.js` 2,415 B，分别压缩后合计 **95,412 B**；两个原文件拼接后一次 gzip 为 **95,498 B**。两种口径都低于 204,800 B 阈值；实际分文件传输采用前者。
- 初始化：5 个全新 context：`20.60 / 14.70 / 14.40 / 15.00 / 16.30 ms`，中位数 **15.00 ms**。
- 解析耗时（ms，三轮中位数；括号内 = wasm / Dart）：
  - 87 字符：0.0367 / 0.0174（≈2.11×）、0.0299 / 0.0171（≈1.75×）、0.0320 / 0.0170（≈1.89×）。
  - 1 万字符：4.13 / 3.01（≈1.37×）、3.80 / 2.81（≈1.35×）、3.93 / 2.63（≈1.50×）。
  - 10 万字符：38.50 / 24.50（≈1.57×）、36.90 / 25.60（≈1.44×）、38.90 / 25.60（≈1.52×）。
- 测试环境中 `spike/md-bench/*.json`、`spike/md-bench/result-*.txt` 与 release 截图 `spike/md-bench/p0-release.png` 可作为复核输入；`firstChecksum` 生效，说明两条路径都真的跑了。

#### 结论与停点状态

- **判定：P0 未达标（性能项高于 1.2× 阈值）。** 不要在本阶段自行决定走退出方案；请审核方按方案把「继续优化 / 进入退出方案」的选择交给用户。
- **停点：G2 待审核。** WP-W 在审核结论前不再推进；如继续优化，建议先确认 JSON / boundary 成本、把 block mapping / JSON 往返与 wasm 解析耗时拆开归因，以及对比 sequential id + 子块索引化后是否回到 1.2× 附近。
- 若决定退出方案（S2「退出方案」）：Web 端保留 Dart 镜像 + 差分棘轮守护，桌面/服务端按单实现推进。

### G2 继续优化后的协调者复核（2026-10-27）

用户已选择继续优化，预算不变。本节是执行方复核材料，不替代 `reviews.md` 的审核结论。实验位于 `refs/heads/ss/w/p0-costs`，基于 S1 候选 `12c7ff2`；核心优化独立提交 `6594f12`，初始实验提交 `8a2af3f`，后续纠错与原始数据在该实验分支保留。没有合并核心或 spike 至 `single-source`。

**推荐 `opt-level=3`。** 最终 Flutter release、同页 Chrome 对照采用 AB/BA 交替批量采样；每轮先求 full / Dart，再取三轮比值中位数。完整路径包含 Rust wire JSON 编码、跨边界字符串传递、Dart JSON 解码及 BlockSpec 映射，没有绕过 JSON 或缓存解析结果。

| 输入 | 实际字符数 | 三轮同轮比值中位数 | 每轮均 ≤1.2 |
| --- | ---: | ---: | --- |
| 原 P0 87 | 87 | 0.787 | 是 |
| 原 P0 10k | 10000 | 0.645 | 是 |
| 原 P0 100k | 100000 | 0.616 | 是 |
| S0 同形短输入 | 87 | 0.771 | 是 |
| 近似 S0 10k | 9997 | 0.685 | 是 |
| 近似 S0 100k | 99972 | 0.651 | 是 |

原 P0 三档逐字节保留；S0 两个长输入按行边界截断，与官方 S0 的 substring 输入不同，只作代表性补测，不能称同输入基线。原 P0 100k 最终 full 中位数 12.75 ms、同轮 Dart 三轮中位数 20.70 ms；表中比值仍按逐轮比值汇总，不由这两个数相除得出。

- **体积**：GNU gzip 1.14 `-n -9` 对 wasm / JS 分别压缩，120538 + 2535 = **123073 B**（120.188 KiB）。低于 204800 B 硬预算，比 122880 B 软目标高 **193 B**。旧 Node zlib 的 123226 B 是拼接压缩，分文件实为 123260 B，不混用口径。
- **初始化**：保留的同一 opt-3 产物五次新上下文中位数 **12.8 ms**，低于 60 ms；最终补测的 served SHA 与体积记录一致，核心 fingerprint 为 `6b193da84a7b630a`。
- **输出与回执**：优化前后七个 host 输入 wire JSON 黄金 SHA 相同；浏览器六组输入全部块与 marks 比较无差异；最终 `receipt: 4 passed (floor 4)`。结构、ignored-source 检查与 shellcheck 通过。该结果不能外推到全部语料、Fragment、Firefox / Safari 或桌面 FFI。
- **收益来源**：`parse_body` 的逐子块查找改为一次索引与所有权移动；Sequential 去掉冗余唯一性集合；wire 去掉块深拷贝与排序时字符串分配。独立复核未发现这三个核心 diff 改变样本输出。
- **UUID 纠错**：旧 +48% 来自 `parse_body` 对 `parse` 的不对称探针，不能当纯 UUID 成本；最终对照统一 `parse`、mode、root 与 digest，仅 BlockIds 不同。测得的是整套 ID 策略差值，仍非单独随机数系统调用耗时。
- **JSON / 绑定归因限制**：历史 base-z 100k 的 echoJson 为 **1.56875 ms**，不是 µs；Dart decode 1.3375 ms、BlockSpec 映射 0.659375 ms。`parseBody − parseOnly` 包含不同 digest / 返回形态，只有粗估意义。**Rust BlockJson 映射与 JSON 编码尚未做孤立计时**，不能把差值当精确阶段耗时，也不能将各阶段相加。
- **汇总纠错**：旧 0.736 / 0.451 / 0.514 使用阶段中位数之比，且旧 runner 固定顺序；历史值更正后保留，最终判读以上述 AB/BA 数据为准。

原始证据在实验分支 `spike/md-bench/summary-final.json`、`sizes-costs.json`、`final-opt3-paired.txt`、`final-opts-short-paired.txt`、`final-ids-paired.txt`、`receipt-final.txt`。体积包含诊断导出，是该 spike 导出集合的实测，不证明正式十个 API 产物体积；正式 S2 需重新验收。

### 独立 JSON 阶段计时复核（2026-10-27）

上述 JSON 归因缺口已补齐：实验分支新增 `spike/md-wasm-p0/src/stage3.rs`，在 Rust 内用 `performance.now()` 分别测 Block → BlockJson 投影/规范化和 `serde_json::to_string`。解析、参考 wire 输出生成、输入 clone 准备及 JS 返回均不计入两段；完整输出在计时后逐字节与真实 wire 比较并计算 checksum。核心代码未再修改。

| 原 P0 输入 | 投影/规范化三轮中位数 ms/次 | JSON 编码三轮中位数 ms/次 |
| --- | ---: | ---: |
| 87 字符 | 0.000585938 | 0.000891113 |
| 10000 字符 | 0.055468750 | 0.089062500 |
| 100000 字符 | 0.550000000 | 0.875000000 |

每档三轮，每轮预热后测 15 个调用；各调用内部批次分别为 8192 / 128 / 16。计时器 call + loop 摊薄中位数约 0.000203 ms，最小非零 tick 约 0.1 ms；利用批量摊薄量化误差，不扣减底噪，不把各阶段相加。探针复制了私有 wire 投影逻辑，仅用于实验；准备和批量驻留改变缓存/内存状态，结果不是生产单次调用可精确相加的账本。

新增回执 **`6 passed (floor 6)`**，含投影镜像与真实 wire 的七输入字节等价、null 归一与稳定排序检查；shellcheck、结构和 ignored-source 门禁通过。实际 served 的诊断 wasm / JS SHA 与 `summary-stage3.json` 对应，核心 fingerprint 仍为 `6b193da84a7b630a`；新增诊断产物不冒充先前 123073 B 的预算产物。原始证据：实验分支的 `summary-stage3.json`、`stage3-opt3-raw.txt`、`receipt-stage3.txt`。

**停点状态：UUID、解析、JSON、绑定成本的实验归因已交付，硬性能门槛已有达标证据，推荐 opt-3，G2 待正式审核。G1 未通过，正式 T2.2+ 未启动，不合入 main、不发版。**

### WP-W 正式绑定与构建（2026-10-27；基于 `8d78ba6c`）

本节是 WP-W 执行记录，历史 P0 不作 M2 正式产物证据。分支 `worktree-agent-aa074de1015a08b4c`；未 merge、未 push、未发布。Dart 门面/VM、浏览器与 Windows 契约入口归 WP-D，核心黄金文件归 WP-R。生成物就绪不等于 WP-D 已接入。

#### 交付范围与提交

- T2.2 `b730945f`：wasm-bindgen 精确 `=0.2.129`；crates.io 版本 API 实查发布日期 `2026-09-25T07:13:40Z`，截至执行日已满两周，且是最新稳定 0.2.x。CLI 同号；依赖理由为 Web 字符串同步绑定，仅 `crates/md-wasm` 直接新增。登记 AGENTS 一行、stable wasm target、release-check 的版本拒绝门禁。
- T2.4 `bc72bc12`：十个 `#[frb(sync)]` 转发、FFI 独立 lock、md-core 指纹 ROOTS。真实运行 FRB **2.12.0** 生成器；为避免无关格式化/依赖升级，使用 `generate --rust-root rust --rust-input crate::api --dart-output lib/src/rust --no-deps-check --no-rust-format --no-auto-upgrade-dependency --stop-on-error`，随后只 rustfmt 生成 Rust 文件。已核对并还原 pub get 的平台插件换行及旧 API 的格式噪音。Dart 生成差异严格为新 `api/markdown.dart` 与三个 `frb_generated*.dart`；Rust 改动为本任务 wrapper/module/build/manifest/lock 和 `frb_generated.rs`。提交统计 10 files、766 insertions、44 deletions；其中 build.rs 仅内容增加两个 ROOTS，另外是工具缩进。
- T2.3 `f97268be`：正式 `crates/md-wasm` 十函数、build.rs、print_fingerprint；workspace member 与 root package profile **opt-level=3/codegen-units=1**。根 lock 的 wasm-bindgen 同族 js-sys/web-sys/wasm-bindgen-futures 和 syn 更新是精确 pin 的解析结果，未更新其他无关包。
- T2.6 `ba92c38a`，后续 `fa08c4ef`：locked 正式构建、分文件 gzip 字节硬门禁、SOURCE_FINGERPRINT 与测试支撑复制；黄金语料缺失或无 JSON 文件时硬失败。当前缺 corpus 的失败是前置依赖未交付，绝不能标成契约通过。
- T2.7 `efc70ba9`：no-modules 在 Flutter 前启动；`wasm_bindgen({module_or_path: ...})`，Context7 官方 no-modules 文档及 0.2.129 生成代码均已核对。Promise 同时捕获脚本缺失导致的同步错误。产物忽略和 ignored-source 显式允许名单已接入。
- T2.9 `90b5f79a`：宿主实际调用十个导出，正文双模式、HTML 双 profile、UTF-16 与 conformance fixtures；新增可复现 HTML 特性探针。用户任务中的“契约测试 T2.11”按本文实际编号 T2.9 记录。
- T2.11 `be466f0b`：just dev-web/build-web、local-ci（包括 Windows）、browser-tests、release-images、CI 三处 Web 构建/浏览器套件、release Web image 全部先构建 wasm。CI 安装同号 CLI；md-core/md-wasm 单独改动能选中 rust/flutter/web-e2e。发布编排 fixture 同步新工具门禁并真实回归通过。
- T2.10：桌面 ROOTS 已随 T2.4 完成；Web freshness 的 Dart 测试归 WP-D，本包提供相同 FNV 输入算法与明文指纹。T2.14 的 Windows CI 固定名单尚未改动：实际 integration_test 文件在本分支不存在，待 WP-D 交付可运行文件后再接入，安装包冒烟也待集成验证。

#### 正式体积与构建口径

正式导出只有 §3.3 十个业务函数（另有 wasm-bindgen 自身 initSync/内存分配等支撑），没有实验诊断导出。`cargo tree -p mica-md-wasm --target wasm32-unknown-unknown -e features --locked` 已确认没有 uuid、render、merman、ratex。宿主示例不进入 cdylib 导出。

首次正式 opt-3/LTO 构建：wasm **221744 B** + JS **2744 B** = **224488 B**，脚本真实拒绝。使用 wasm-bindgen 内置 `--remove-name-section --remove-producers-section` 后，同产物为 208974 + 2744 = **211718 B**，仍拒绝。最终为本次 wasm 构建的所有依赖统一单 codegen unit：

```
CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_PANIC=abort cargo build -p mica-md-wasm --target wasm32-unknown-unknown --release --locked
wasm-bindgen --target no-modules --no-typescript --remove-name-section --remove-producers-section ...
```

这些环境变量只作用该条脚本命令，未改变全仓库 profile、工具持久配置或其他构建。root package opt-3 未降低。`rustc --print cfg --target wasm32-unknown-unknown` 实际输出 `panic="abort"`，显式设置与该目标默认一致。strip 是 wasm-bindgen 内置参数，无新增 wasm-opt/其他依赖。

最终 GNU gzip `-n -9 -c` **分别**压缩后相加：wasm **196936 B** + JS **2744 B** = **199680 B（195 KiB）**；低于 204800 B 硬门槛，高于 122880 B 软目标。减小来自删除元数据与统一依赖 codegen，未删除 API、未换压缩口径。原首次失败日志在任务输出 `bk1q1kr2f.output`；正式成功及严格 gold 缺失失败日志保存在 `9e604c43:scripts/md-wasm-evidence/`。首次未 strip 原产物已被后续构建替换，因此不能声称保留其二进制哈希；strip 中间文件还在 `target/md-size-check/`，SHA `e792a6e7ade84d76a433f5ec816dd9b19115dd4c042287e6f98eb559fa227e91`。

测量时的正式文件 SHA-256：

- wasm `29156406756c08566da085b6a9864b87b34ce8444467bef6ea16a8d594ccf878`
- JS `90ee81fdb2466b0e2a497216203aa27907cc42777997f27b55b6141634d9a532`
- Web engine fingerprint `c9c66838431d7082`

Web engineInfo 保持核心 crate_version/schema_version，仅 source_fingerprint 为 md-core+md-wasm（含 wasm build.rs）的实际源集指纹。核心/桌面指纹源集不同是构建信息的预期差异；协调者已确认不改变冻结字段形状/含义。两个 build 目标和 test/support 的文件一致；后续源码变更必须重新构建，不可沿用这些数字。

#### M2 Web 真实 release 测量

Windows 11、Rust 1.98.0 stable、Flutter 3.47.1/Dart 3.13.1、Chrome 156；`flutter build web --release --no-web-resources-cdn --target=../../scripts/md-wasm-bench.dart --output=build/md_wasm_bench`。`scripts/md-wasm-static.cjs` 返回 no-store、application/wasm，没有 COOP/COEP，实测 crossOriginIsolated=false。Playwright 真实浏览器调用编译后的 Dart `runMdM2`；未用 JS 解析替代 Dart 解码。完整路径包含 Rust wire JSON 编码、边界字符串、Dart jsonDecode 和 BlockSpec 映射；同页对照现有 markdownToBlocks。三轮，每轮预热、15 批 AB/BA 交替；批次 1024/8/1。每轮取各路径批次中位数之比，再汇总三轮比值中位数，原始每批样本全部保留。

| 精确输入 | 三轮 full/Dart | 三轮比值中位数 | 预算 |
| --- | --- | ---: | --- |
| S0 hotPath87（87 UTF-16） | 0.75618 / 0.76527 / 0.78827 | **0.76527** | 每轮 ≤1.2，通过 |
| S0 fixture substring 10000 | 0.52117 / 0.53505 / 0.53645 | **0.53505** | 每轮 ≤1.2，通过 |
| S0 fixture substring 100000 | 0.57071 / 0.56747 / 0.56649 | **0.56747** | 每轮 ≤1.2，通过 |

这是同页 production dart2js 对照，不与 S0 dartdevc 的绝对值直接相除。最终逐轮 full 中位数为短档 0.014844/0.014648/0.014550 ms，10k 2.925125/2.862500/2.849875 ms，100k 33.101/32.800/32.800 ms。先前同产物预热不同的首轮测量仍完整保留于 browser-raw，不用挑选其中更好数覆盖 final。Dart 页面 SHA 和正式 wasm/JS SHA 见 browser-final 的 manifest；最终 full 测量与 diff 导出为同一个 release 页面清单。

五个新 browser context，micaMdReady 赋值到 Promise 完成（包含 fetch/instantiate）为 **20.6 / 12.7 / 12.2 / 12.0 / 25.6 ms**，p50 **12.7 ms**，全部 ≤60ms。测量用初始化前 setter 记录 Promise fulfillment；另记录 instantiateStreaming 调用起点，不把轮询观察延迟算入。浏览器复用进程/编译缓存、新上下文，不代表冷机器或真实网络。初始化用同份 wasm/JS、Dart 测量入口尚未加 diff 函数，差异仅页面 JS。首帧推迟与完整应用启动、桌面 p50 未在此独立页面证明。

浏览器分别对 parseInlineBatch/decodeCells/encodeCells/serializeBody/serializeInline 的畸形 JSON 和 renderHtmlBody 的非法 profile 真实调用，全部抛 `Error: invalid input: ...`；随后同实例 parseBody('# hello') 正常。Node 也补测通过，但浏览器证据单独保留，不以 Node 代表浏览器。没有将 panic/错误吞成成功。

语义证据：原始 10k/100k `equivalent=false` 保留。精确导出发现分别只有 2/30 个块 marks 顺序不同，首例块 169/175：同 UTF-16 范围 image/link 的产生顺序与 wire 规范顺序不同。**非语法漂移，是 §3.4 契约规定的标记排序规范化，Dart mirror 顺序未排序**。按 (start,end,type) 排序后全部块 differingBlocks=0，kind/text/data 值不变。S0 比较器原本就在 `markdown_differential_test.dart:301–318` 这样排序，故不是新增字段分歧。三档同输入通过真实 host 无 render wire_stdin 与浏览器 wire **逐字节相同**（SHA 保留）。10k 尾为 `sta`，100k 尾为 `~~~a~`，没有额外尾截断差异，不删除这两档。原始输入、完整首例与重复块差异、每条 SHA 位于 evidence 的 inputs/diff-summary；完整输入+两侧输出另在 `target/md-m2-diff-inputs.json`。

原始数据及复核入口：`9e604c43:scripts/md-wasm-evidence/{md-m2-browser-final.json,md-m2-browser-raw.json,md-m2-diff-summary.json,inputs.json,html-feature-summary.json,md-runtime-errors.json}`；测量代码 `scripts/md-wasm-bench.dart`，host 复核 `crates/md-wasm/examples/wire_stdin.rs` + `scripts/md-wasm-diff.cjs`。运行时临时数据在 `target/`，截图 `target/md-m2-browser.png`。测量仅覆盖 parse Document 与 Chrome，不外推 Fragment、Firefox/Safari、序列化/HTML 或桌面性能；Firefox 补测未做。

#### 回执、阻塞与裁决请求

- 宿主 `cargo test --locked -p mica-md-wasm`：**10 passed**，零跳过；forward 覆盖十个实际导出。原始回执 evidence/md-forward-final.txt。shell source 回执包装曾被 worktree 安全 hook 拒绝，未绕过；直接 cargo 原始通过数已留存。
- `cargo clippy --locked -p mica-md-wasm --all-targets -- -D warnings`：**未通过**，核心既存 no-default-features 路径 `ids.rs:20 id_for` dead_code。已报告 WP-R，等待其 cfg 修复提交，不加 allow 掩盖。
- 桌面 `cargo build --release --locked --manifest-path clients/mica_flutter/rust/Cargo.toml`：exit0；`cloud_sync_seed_healthy_test.dart`：**2 passed**，零跳过，JSON `target/t24-flutter-receipt.json`。
- 根 `cargo fmt --check`、新 crate 格式、结构/ignored-source/ci-paths、gfonts mirror、release orchestration fixture：通过。新构建脚本 shellcheck -x 通过；批量检查另有既存 ci-paths 间接变量 SC2034/source 搜索路径告警，未谎称全仓 shellcheck 绿。
- 独立 FFI crate 全库 `cargo fmt --check` 是**既存基线失败**（旧 API 四空格与根 rustfmt 两空格配置），不是本任务引入；新文件和生成物已格式化，没有提交全库噪音。
- 严格 gold 缺失门禁实跑 exit1，明确 `REFUSED: wire gold corpus missing ... M2 contracts are pending, not passed`。待 WP-R 黄金文件和 clippy 提交导入后重跑。
- VM/浏览器/Windows 门面契约、Windows 安装包冒烟、桌面初始化、完整应用首帧仍待 WP-D 及集成验证。未声称完整 M2 或 S2 已通过。
- T2.13 材料整理后，普通 `git diff --check` 被 RTK/worktree 安全 hook 明确拒绝；按协调者最新约束停止后续 Git 操作，没有换入口绕过、没有请求主会话代跑。因此本节、测量脚本/evidence 与 wire_stdin 示例目前留在工作区，**尚未提交**。已提交的最后一项为 `fa08c4ef`；已提交与未提交材料须分别审核。

**冻结契约冲突申请（未自行改 core）**：真实 FFI cargo tree 显示 `mica-markdown/render` 统一打开 `mica-md/render`；wasm 刻意关闭。当前 api.rs 的 HtmlProfile::Clipboard 仍与 Export 共用忽略 profile 的 export_html。`html_feature_probe` 同输入 `$x$` 在 off/on 的 export 与 clipboard 均为 **34 vs 2128 B**（span vs SVG）；Mermaid `graph TD; A-->B` 为 **68 vs 9342 B**（code vs SVG）。两端 HTML 不逐字节相同；gold 不能跳过这些输入以伪造一致。原始完整输出 `target/html-feature-{off,on}.txt`，summary 记录 SHA。可复现命令为 `cargo run --locked -q -p mica-md-wasm --example html_feature_probe` 与再加 `--features mica-md/render`。

请求审核方裁决：提前落地 S3 的 Clipboard 独立语义并明确 Export 平台差异，或调整 wire HTML 的独立于 render-feature 的实现规则。直接关闭 FFI render 会破坏已有本地导出；给 wasm 打开 render 会违反目前依赖/体积约束，均不建议。本包未更改冻结公共 API/wire/Dart 接口，未为避开差异跳过 HTML 测试。

#### 后续协调补充：对象键顺序（未提交、未重跑门禁）

协调者最终核实 WP-R 提交 `881502aa`（ids cfg）及 `7a9ee622`（wire_gold 与九个 fixture，替代初版 `992ab8a`）。截至本次证据快照，本工作区未接入这些提交，严格 Clippy 与包含黄金文件的正式构建均未重跑，不标通过。固定默认参数重放的协调结果为 parse_body 58/76 与 encode_cells 5/11 仅对象键顺序差异，HTML 8/154 有实际内容差异，其余七个函数固定输入逐字节相同；render 整文件 3 passed / 5 failed 含输入生成差异，不能直接解读为五个函数发生输出漂移。

WP-W 通过允许的非 Git 命令再次核对真实依赖图：`cargo tree --manifest-path clients/mica_flutter/rust/Cargo.toml --locked -e features -i serde_json` 显示目标运行时的 `serde_json/preserve_order` 由 dugong、dugong-graphlib、merman-core、merman-render 开启，并统一进入 FFI 使用的 serde_json；对应 `cargo tree --manifest-path Cargo.toml --locked -p mica-md-wasm --target wasm32-unknown-unknown -e features -i serde_json` 只有 default/std，没有 preserve_order。这是除 HTML 输出之外的独立字节一致性风险：Value::Object 的序列化键顺序受构建特性影响。

此前三档“host 与 WASM 逐字节一致”的证据限定为**无 render 的 host 与正式 WASM**，不能外推为真实桌面 FFI 与 WASM 逐字节一致。语义 canonical 比较通过也不能替代字节目标；跨特性的 byte 失败与 canonical 结果由 WP-R 分别保留，WP-W 尚未补跑真实 FFI 对照。本记录不以排序测试掩盖失败，也不擅改冻结契约。

#### 工具恢复后的证据归档（2026-10-27）

用户暂停 RTK hook 改写、会话重新读取配置后，按协调者要求仅用普通 `git status --short` 重验成功，随后 `git diff --check` 也通过。未修改任何持久 hook/权限配置。此前“未提交”条目保留为中断经过；T2.13 脚本、完整输入/两侧输出原始数据与本记录已归档为 **`452b8c4a`**，随后将 WP-R `881502aa` / `7a9ee622` 分别 cherry-pick 为 **`c02bd94f` / `2c2fd65e`**。未合并集成分支、未 push。

#### 接入最终核心修复与黄金语料后的重验（2026-10-27）

本节取代上文“未接入/未重跑”的当前状态，但保留历史失败。新的直接 Cargo 命令及原始日志均在 `9e604c43:scripts/md-wasm-evidence/md-post-*.txt`：

| 命令 | 实际结果 |
| --- | --- |
| `cargo test --locked -p mica-md-wasm` | exit0；forward **10 passed / 0 failed / 0 ignored**，lib/doc 空套件另显示 0，不混算 |
| `cargo test --locked -p mica-md --test wire_gold` | exit0；**9 passed / 0 failed / 0 ignored** |
| `cargo test --locked -p mica-md --no-default-features --test wire_gold` | exit0；**9 passed / 0 failed / 0 ignored** |
| `cargo clippy --locked -p mica-md --no-default-features --all-targets -- -D warnings` | exit0 |
| `cargo clippy --locked -p mica-md-wasm --all-targets -- -D warnings` | exit0 |
| `cargo fmt --check` | exit0 |
| `cargo build --release --locked --manifest-path clients/mica_flutter/rust/Cargo.toml` | exit0（共享核心改动后重建） |
| `bash scripts/build-md-wasm.sh` | exit0；严格黄金存在性/非空检查通过，九个 JSON 已复制 |
| `diff -r crates/md-core/tests/fixtures/wire clients/mica_flutter/test/support/mica_md/gold` | exit0；复制逐字节相同 |
| `just check-structure`、ignored-source、ci-paths、diff-check | exit0 |

`source lib-receipt + 分组 + tee` 的组合命令仍被独立 worktree 安全检查拒绝（与 RTK 不同），没有执行该 wrapper；按检查器明确建议运行直接 Cargo 命令并保留完整 `test result` 行。因此上述通过数是直接命令的实际计数，**不是未执行 wrapper 的回执**。没有为隐藏 source 编写 shell 包装，也未改变权限。

正式严格构建新产物：**196937 B wasm + 2744 B JS = 199681 B gzip -n -9**（硬门槛 204800 B，通过）。WASM SHA-256 **`9df6253d1181d7029a4544b25600945278d9cf4750a1db18ca46da40e54f9779`**，JS SHA 仍为 `90ee81fdb2466b0e2a497216203aa27907cc42777997f27b55b6141634d9a532`，引擎指纹 **`7f78019aa20d7d15`**。原产物 199680 B 与旧指纹数据留存；没有凭修复意图宣称字节不变。指纹输入仅 md-core/md-wasm 清单和源码，不含 scripts 的原始证据。

因 WASM hash 改变，换入已有 release 页面并独立重测，Dart `main.dart.js` SHA 保持 `5f9c0ff68854e1d2e29745c37fed05f030aef2246b8d2588e5f5a63467bd03aa`。仍为真实 Chrome 156、同页 production dart2js、原三档输入、每轮预热+15批 AB/BA、三轮，无额外矩阵：

| 输入 | 新产物三轮 full/Dart | 比值中位数 |
| --- | --- | ---: |
| 87 | 0.968293 / 0.979730 / 1.000000 | **0.979730** |
| 10000 | 0.491837 / 0.517094 / 0.512035 | **0.512035** |
| 100000 | 0.558029 / 0.552092 / 0.549126 | **0.552092** |

每轮均 ≤1.2。五个新 browser context 初始化（Promise 赋值到 fulfillment，同方法）为 **35.1 / 13.7 / 11.4 / 13.3 / 12.3 ms**，p50 **13.3 ms**；crossOriginIsolated=false。六类非法 JSON/profile 在新产物浏览器均抛 Error，随后同实例 parseBody 正常。新清单、全部原始样本、初始化与异常记录在 **`9e604c43:scripts/md-wasm-evidence/md-m2-post-integration.json`**；旧 browser-final/raw 文件未覆盖。临时截图 `target/md-m2-post-integration.png`；静态服务器测完已停止。

新增 `node scripts/test-md-wasm-gold.cjs` 直接加载正式 wasm，对 checked-in 默认黄金的返回字符串逐字节重放：parse_body **76**、parse_inline_batch **15**、decode_cells **28**、encode_cells **11**、serialize_body **77**、serialize_inline **9**、render_html_body **154**、math_run_spans **13**、parse_inline_math **13**，合计 **9 文件 / 396 cases** 全部通过。engineInfo 单独断言 schema_version 与该 WASM SOURCE_FINGERPRINT 一致（不与核心不同源集指纹强行相等）；六类错误+随后恢复也通过。该 Node 专项验证**不能替代 Dart VM/web/Windows 契约，也不能证明实际 FFI render 字节相同**。

当前剩余阻塞不变：render HTML 因 `cfg` 不在 wire_gold render 测试里执行，默认 9 绿不能称真实 FFI 全九项绿；固定默认 args 的 HTML 实际内容差异、parse_body/encode_cells 对象键序差异继续待审核裁决。未改测试去掩盖这些失败。WP-D 门面与 Windows 实际集成/首帧/安装包验证仍不在本工作包完成证明中。

### 2026-10-27 WP-D：Dart 门面与真实 FFI 接线

基线 `8d78ba6c`，分支 `worktree-agent-a64add55c7b76fbd3`。本记录是执行证据，不是 M2 通过结论。未修改冻结接口、未删除 Dart 镜像、未推送或发版。

- **T2.5**：`Mark` 原样迁移、旧 `marks.dart` re-export；`BlockSpec` 迁到冻结门面，旧解析器 re-export。接口、异常、访问器、初始化兜底、不可用实现、wire 映射和 wasm 实现已落地。依赖实际 `bc72bc12` 生成物实现十个同步 FFI 调用；仅检查真实初始化状态来容忍已初始化，其他初始化失败继续抛给门面兜底。提交 `50bf6030`、`69f92edd`、`0281dc34`。旧镜像的格式化噪音已撤回。
- **T2.8**：`main()` 在 `runApp` 前 await 初始化。`main.dart` +4/-4，净 **0 行**，同库 18 文件仍为 **21088 行**；合并两处原注释抵消 import 与 await。提交 `e3b227ad`。
- **T2.9 Dart 侧**：真 FFI 的 `useNativeMarkdownCodec()` 先执行 `initCheckedNativeRust()`，再安装 io codec；共用断言、VM/Web/Windows 入口、文件系统 gold 查找和浏览器 fetch/wasm loader 已落地，缺文件失败。提交 `a0f2c0c3`、`45ba2e17`。黄金文件原始 output 保留，块和 marks 值按结构比较、Markdown/HTML 按字符串比较；不排序 marks、不清理 data 字段。Dart 门面没有 export HTML API，因此 export 用例不计为 Dart 通过/跳过，由 wire/绑定层另测；跨端字节与 render 一致性尚未确立。
- **降级测试**：所有不可用方法均抛 `MarkdownCodecException`，未初始化访问器报 `StateError`；另加浏览器缺 wasm 的真实初始化失败后仍可绘制首帧的测试（尚未执行）。提交 `66f28d78` 的 T2.12 标签沿用了初始委派编号，**不表示下述真正 T2.12 已完成**。
- **阶段文档 T2.12（差分工具切门面）未完成**：保留原 S0 Rust `MicaDocument` 路径及基线，未再生成 differential baseline；该改造不能用降级测试替代。**T2.10 Web freshness 不属本轮已完成项**，本分支没有新增 freshness 测试或 wasm 指纹门禁。
- **Windows 入口**：提交 `44a4716d`，同一 gold body 供 `integration_test/markdown_codec_test.dart` 使用；另一个启动入口复用真实 `app.main()`。初始委派称 T2.13 desktop，阶段文档中 Windows 验证实际为 **T2.14**；阶段 T2.13 性能预算本轮未验收。

接入依赖（只 cherry-pick，不手改生成物）：`6633f7e7` = WP-W `bc72bc12`、`74550843` = WP-R gold `7a9ee622`、`a89113cf` = `881502aa`。保留现有 render 特性，未绕过它来使黄金测试变绿。

#### 命令与验证结果

以下路径相对 `clients/mica_flutter`，JSON reporter 均在忽略的 `build/` 中。`scripts/lib-receipt.sh` 的 source 入口仍被 worktree 隔离检查拒绝（无法判定 source 的 Git 作用域），故不冒称正式 receipt wrapper 已执行；通过数来自 Flutter JSON runner 中 hidden=false/skipped=false 的 testDone。

| 项 | 实际命令 / 证据 | 结果 |
| --- | --- | --- |
| 新鲜原生库 | 根目录 `cargo build --release --manifest-path clients/mica_flutter/rust/Cargo.toml --locked` | exit 0；契约和差分测试均先验源码指纹，ROOTS 含 md-core |
| 真实 VM gold | `flutter test --file-reporter json:build/wp-d-contract-receipt.json test/markdown_codec_contract_test.dart` | **8 passed / 1 failed / 0 skipped**，exit 1（添加身份断言前的九组回执） |
| 新增身份覆盖 | `flutter test --plain-name 'MarkdownCodec engine identity and single-inline wrapper' --file-reporter json:build/wp-d-identity-receipt.json test/markdown_codec_contract_test.dart` | **1 passed / 0 failed / 0 skipped**，exit 0；真实 engineInfo 与单条 wrapper |
| 功能回归 | `flutter test --file-reporter json:build/wp-d-regression-receipt.json test/markdown_codec_unavailable_test.dart test/markdown_wire_test.dart test/markdown_gold_parser_test.dart test/markdown_differential_test.dart test/mark_title_test.dart` | **12 passed / 0 failed / 0 skipped**，exit 0；其中 S0 差分 3 项，原基线不退化 |
| 全量静态分析 | `flutter analyze --no-fatal-infos` | exit 0，164 条存量 info；不宣称无提示。新增门面/测试目标范围单独 analyze 无问题，exit 0 |
| 结构 | 根目录 `just check-structure`、`bash scripts/check-file-size.sh --diff 8d78ba6c` | 均 exit 0，main 净 0 |
| Windows 契约 | `flutter test --file-reporter json:build/wp-d-windows-receipt.json integration_test/markdown_codec_test.dart -d windows` | Debug 构建 **646.6 s** 成功；随后连接失败，**0 tests executed**，加载错误 exit 1 |

唯一真实 gold 失败是 `render_html_body/23-math.md/document/clipboard`：默认 gold 期望 `<span class="math">E = mc^2</span>` 等源码 HTML；原生 render 返回 `<svg ...>`，首处差异 offset 17。其余八组 parse/body/inline、cells、serialize、math spans 均通过。没有修改期望、跳过此失败或把结构比较当成 wire 逐字节证明；冻结 Clipboard 要求与现有 render 的矛盾交由协调/审核裁决。

Windows 可执行文件为 `build/windows/x64/runner/Debug/mica_flutter.exe`。runner 报 `Error waiting for a debug connection: The log reader stopped unexpectedly, or never started.`，测试未进入 setUpAll。单实例 `Local\\MicaSingleInstance` 只是源码支持的未证实假设；读取现有进程的 PowerShell 命令亦被隔离拒绝，未换工具或终止用户进程。**app.main 启动探针、真实桌面不可用首帧、Windows release 五次初始化 p50 ≤30 ms、安装包冒烟、截图、性能预算、Web 运行与交付均未验证**。646.6 s 是构建时间，绝不当初始化时间。

本轮运行日志在会话 task 文件 `broc7v0s4.output`（gold）、`bkhn0iapj.output`（identity）、`bdndpjhr7.output`（regression）、`bny84ry46.output`（Windows）、`be0k86s3w.output`（analysis）、`by3patbeh.output`（structure）；完整目录 `C:/Users/willz/AppData/Local/Temp/claude/D--codes-mica/6ae5127a-2bb8-4319-979e-679ac1bd3fd3/tasks/`。六个 linux/windows 插件生成文件仅换行元数据噪音，未加入任何任务提交。

### 2026-10-27 WP-D：补齐真正的 T2.10 与 T2.12

本节更新上一节中 T2.10/T2.12 的未完成状态；不改变先前 HTML、Windows 或性能阻塞结论。

**T2.10（`d89db2a3`）**：新增 VM `test/web_engine_fresh_test.dart`。从正式 `crates/md-wasm/build.rs` 的 ROOTS 声明动态读取源码集，用既有 `sourceFingerprint` 计算原字节 FNV；每根先确认存在，缺失失败。无 stamp 明确 skip（不当门禁通过），有 stamp 必须一致。回归测试复制本工作树的真实源码集到临时目录，验证错误 stamp、源码变动、缺失源码会失败，恢复原字节通过，不修改产品源码凑结果。

为具备真实源码依赖，仅接入 WP-W `f97268be`（本分支 `39e90cad`），并接入协调者定向 LF 修复 `ea603749`（本分支 `b6c8d516`）。初次复制 W 正式产物（wasm SHA `9df6253d1181d7029a4544b25600945278d9cf4750a1db18ca46da40e54f9779`，JS SHA `90ee81fdb2466b0e2a497216203aa27907cc42777997f27b55b6141634d9a532`）后，测试真实拒绝 stamp `7f78019aa20d7d15`：本地算出 `c734b1a5bc380b7f`（2 passed / 1 failed / 0 skipped）。五 ROOTS 共 23 文件逐 SHA 对比，仅 `crates/md-wasm/{Cargo.toml,build.rs,src/lib.rs}` 有差，分别本地 17/49/64 个 CRLF，W 为 LF；去 CR 后字节完全相同，其余 20 文件 SHA 一致。这是检出换行影响严格原字节指纹，不是引擎语义 bug。

随后按正式 `CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_PANIC=abort cargo build -p mica-md-wasm --target wasm32-unknown-unknown --release --locked` 重建本树，执行同号 `wasm-bindgen 0.2.129 --target no-modules --no-typescript --remove-name-section --remove-producers-section` 重建 glue，并由 `cargo run --locked -q -p mica-md-wasm --example print_fingerprint` 生成同构建 stamp，取得 CRLF 本树 **3/0/0** 真通过（`bqd0htn7b.output`）。这不是 W 性能证据的替代。最后接入三条 LF 属性，从索引定向物化这三条干净路径（无全仓 renormalize、无算法规范化）；再次使用完整 W 三产物，本地真实源码重算得到 `7f78019aa20d7d15`，**3 passed / 0 failed / 0 skipped**（`b4768abbt.output`；`build/wp-d-t210-lf-receipt.json`）。没有只改 stamp。新增 `.gitattributes` 是协调者提交，D 未改共享源码内容；三个源文件仅 stat 噪音不加入 D 提交。正式 bundle 当前未在本分支接入 ignore 规则，产物显示 untracked，明确不 add。

**T2.12（`24de5e39`）**：Rust 侧全改为真 `markdownCodec.parseBlocks` / `serializeBlocks`，删除差分工具的 `MicaDocument` 通道及 CRDT 归类；新增行内 parse/serialize 比较。只在独立 `markdown_differential_transition_audit.dart` 中保留旧 CRDT 路径，以证明旧新基线迁移来源；它读固定审计 fixture，不是正式对拍通道。原基线归档 `test/fixtures/differential/s0_baseline.json`，逐 id/category/field 旧新值及原因在 `t212_transition.json`，真实 raw/CRDT 前后结构在 `t212_crdt_audit.json`，数据仅来自固定仓库公共语料与 seed fuzz，无账号或用户文档。

完整 **5742 个输入**：旧 4336 条 → 新 **5305 条**。`crdt-normalization` **227 → 0**；旧维度的总记录 4336 → 3749，新增行内测量 1556 单列，不当作新增引擎回归。

| 转移类别 | 条数 | 依据 |
| --- | ---: | --- |
| 新测量 inline:export | 1520 | 新维度，含转义策略差异；最小例 `spec:11` 字面星号 `*` 与 `\\*` |
| 新测量 inline:text / inline:marks | 各 18 | `spec:493` 的反斜杠链接边界 + 17 个固定 fuzz 定界符边界；完整键和值入档 |
| 原维度新增 export | 2 | `spec:484`、`spec:487` 空链接：raw wire 保留零宽链接，旧 CRDT 导出空；实际两路径审计证明，不是绑定丢字段 |
| 原维度新增 Rust roundtrip | 72 | 原始重叠/分段 marks 被旧 CRDT 规范化后掩盖；41 个相关输入 raw vs legacy CRDT 结构入档，新增项逐组审计通过 |
| 消失的旧记录 | 661 | crdt-normalization 227、export 180、marks 6、roundtrip 248；逐项旧值及消失原因入档 |
| 同 key 内容变化：仅对象键序 | 18 | block-count 17 + roundtrip 1，结构完全等价 |
| 同 key 内容变化：实际 CRDT 值变化 | 93 | export 58、marks 15、roundtrip 20；export 逐项重走旧 fromBlocksJson 对上旧报告、raw 对上新报告；其余输入 raw→CRDT 与旧 fromMarkdown 结构相等，审计真通过 |

迁移算式：`4336 + 1556 + 74 - 661 = 5305`。没有因为数量变化修改引擎/语法/HTML。新的 `s2-codec-full-values-v1` ratchet 同时保存键与**未截断完整值**，比较只按冻结允许的对象键序/marks 排序；自检证明相同 id/category/field 下第 300 字以后变化仍失败，避免旧 key-only 门禁漏掉 111 项值变化的问题。先跑旧基线得 **2 passed / 1 failed** 并保留新增证据，完成审计后获协调同意生成新协议基线；随后**非更新模式**跑完整语料及审计 **5 passed / 0 failed / 0 skipped**，exit 0（`bxl3nz1og.output`；`build/wp-d-t212-final-receipt.json`）。生成模式回执另存 `wp-d-t212-generate-receipt.json`，不拿它替代非更新证明。

最终定向 analyze 三个改动测试文件无问题，exit 0；`just check-structure` 和 `bash scripts/check-file-size.sh --diff 8d78ba6c` 均 exit 0（`blygrx4yj.output`），main 仍净 0。正式 source receipt wrapper 仍未执行，回执为普通 Flutter JSON reporter。Windows 不重复构建；HTML 与 wire 字节冻结问题仍待审核，**不声称 S2/M2 通过**。

#### T2.11 构建链复审修复（2026-10-27）

恢复工作后复审发现三处实际缺口：`just app chrome` 没有构建 wasm；`release-check.sh` 虽校验 CLI 版本，但 Flutter freshness 测试前可能没有产物而跳过；`build-md-wasm.sh` 仅要求黄金目录非空，没有证明九个文件的内容仍与核心相符。现补齐 Chrome/web-server 启动前置、release-check 的 wasm 构建前置；正式构建先以 `cargo_receipt 9 cargo test --locked -p mica-md --no-default-features --test wire_gold` **验证** checked-in 默认黄金，再构建/复制，拒绝 `GEN_GOLD=1`，不在生产构建生成黄金文件。

新增轻量 `scripts/test-md-wasm-build.sh` 以临时隔离工具链 fixture 执行真实 build 脚本，不构建 Rust/Flutter、不改用户配置。实际通过：CLI 错号、gold 测试失败、通过数低于 9、生成模式、空语料均拒绝；成功路径验证 locked 构建参数、指纹与 gold 复制；真实 `just --dry-run app chrome` 包含 wasm 前置。CI paths 作业接入此轻量回归。release orchestration fixture 的 Flutter test stub 额外要求 wasm-ready 标记已存在，顺序回归实际 exit0；ci-paths 现有真实回归也 exit0。无临时个人绝对路径写入脚本。

复审同时明确旧 `scripts/md-wasm-bench.dart::_full` 自行调用 wasm 并映射 BlockSpec，历史数据只证明 wire 完整路径预算，**不能称已测真实 WP-D MarkdownCodec 门面**。后续基准改用真实 `initMarkdownCodec` 与 `markdownCodec.parseBlocks`；R 新核心交付后才进行一轮新产物 M2 测量，旧数据保留不冒充。Windows CI 仍等实际测试文件与真实运行结果，不提前登记不存在入口。

构建链修复已提交 `f5ab614e`，加强“先 verify 才 build”行为断言为 `dc81fa1b`。修复后实际 build 脚本先得 **9 passed / receipt 9 (floor 9)** 再构建 wasm，当前旧核心仍为 199681 B；随后正式 WASM 396-case 重放、structure/ignored-source 均 exit0。该次是构建链验证，没有重复旧核心浏览器性能矩阵。

WP-W 仅为编译基准接入 D 的最小官方门面依赖：`50bf6030`→`176a679b`、`69f92edd`→`9d7ba128`、`0281dc34`→`8a6cd7e1`，及 LF 属性 `b6c8d516`→`acf3ed3f`。没有 merge/pull 共享主分支，也未修改 D 的 lib 实现。基准 `_full` 现在只执行 `markdownCodec.parseBlocks`，初始化执行 `initMarkdownCodec` 并读取 engineInfo，若降级不可用则立即失败；保留直接 `micaMd.parseBody` 仅供不计时字节诊断。生产 classic-script loader 初始化 lexical `wasm_bindgen` 并公开 micaMd/micaMdReady，Dart 不直接 `@JS('wasm_bindgen')`。输出记录实际门面路径和 facade_engine，旧测量记录未改写。host diff runner 同时去掉写死 `.exe` 的 Windows 假设。

脚本格式与 Node 语法检查通过；根 scripts 下直接 `dart analyze` 无法解析客户端 package 配置（Flutter/Mica package URI 均不存在），该次 exit3 已如实报告，不当作编译通过。正式 Flutter target 编译与 M2 新产物测量安排在 R 核心修复最终交付后一次执行；当前不占用 D 独占的 Windows runner/VM 工具。

### 2026-10-27 WP-D：恢复桌面/Web 真实运行验证（旧 core）

用户授权继续排除验收阻塞。本节结果绑定尚未修复 Clipboard 的旧核心，不代替最终集成头复测。

- Windows 初始加载失败未证实由单实例造成：只读 runner 见 guard 在 console 初始化前可直接退出；但自有 Debug executable 在隔离 APPDATA 启动后存活超过 10 秒，没有提前 exit，随后仅停止该自有子进程。未读取被拒的进程表、未关闭用户未知实例，也未修改 runner/CMake。
- 最小真实启动路径成功：在 `clients/mica_flutter` 下创建自己的 `build/wp-d-isolated-appdata`，执行 `APPDATA='<worktree>/clients/mica_flutter/build/wp-d-isolated-appdata' LOCALAPPDATA='<同目录>' flutter test --no-pub -v --file-reporter json:build/wp-d-startup-receipt.json integration_test/markdown_codec_startup_test.dart -d windows`。实际环境路径用 Windows 反斜杠传递；启动前 `initCheckedNativeRust()` 校验实际 DLL，调用真实 `app.main()`，有界 pump 等待 `MicaApp` 首帧，并断言 engineInfo 与真实 parseInline。结果 **1 passed / 0 failed / 0 skipped，exit 0**，日志 `b48rhhqv1.output` 有 VM Service 连接和 `markdown codec ready in 0 ms`。该 0 ms 是已初始化 DLL 的 Debug 热路径，**不是 release 初始化预算证据**。隔离目录仅在本工作树 `build/`，可在无自有运行进程后删除该精确目录；不触碰用户原 APPDATA。
- Chrome 真实 wasm 门面首次执行暴露测试 loader 的 bug：正式 wasm-bindgen no-modules glue 顶层是 `let wasm_bindgen`，Dart `@JS('wasm_bindgen')` 读取 globalThis 属性，报 `NoSuchMethodError: tried to call a non-function ... dart.global.wasm_bindgen`（`br4sb7mha.output`）。修复 `aec1b1ff` 仅改测试 loader，用 classic script 如生产 index 初始化，promise 真实 await 并通过 globalThis.micaMd 调用门面。后续真实 wasm 九组 gold + engineInfo/wrapper + 缺引擎首次绘制，共 **11 passed / 0 failed / 0 skipped**，exit 0（`byjxl1h5r.output`、`build/wp-d-web-contract-receipt.json`）。不改 glue/core，不以 DOM eval 实现产品逻辑。
- `19cd0ed4` 新增同模式性能与分段诊断，输入复用 S0 的精确 code-unit substring、controller 选区。旧 core 初次 VM 三轮中位比：parse87 **1.396（失败）**，parse10k 0.651、parse100k 0.515、serialize10k 1.152 通过；html10k **4.811（失败，旧 render 公式 SVG 路径）**。原 JSON `build/wp-d-vm-codec-bench.json`，测试明确失败而非放宽预算。后续已补等量未计时 VM 预热，最终验收应待新核心同输入复测；不混用 Debug VM / Windows AOT / Chrome 的比值。
- 87 字符独立诊断：Rust parse+wire JSON+FFI **0.015718 ms**，Dart JSON decode **0.002301 ms**，block map **0.000514 ms**，decode+map **0.002533 ms**，完整门面 **0.019711 ms**。它们独立采样、不可直接相加减当精确成本；原生 parse-only 归因由 WP-R 探针处理。证据 `bvq0ad4s5.output` / `build/wp-d-boundary-bench.json`，诊断测试 **1 passed**。数据不掩盖 1.2× 预算失败。

release 五次冷进程和实际缺 DLL 后应用首帧的独立 probe 已准备，等待 WP-R 修复后的稳定 core 再构建/测量；不提前声称 release 初始化/性能通过。


### WP-R · 2026-10-27 · S2 wire 字节一致性与提前实现 T3.4 Clipboard

- 基线分支：`worktree-agent-aaa10db22e269aaf4@7a9ee622`；本轮仅改 `crates/md-core/**` 与本执行记录，未改冻结 02 / reviews / API / 消费方。
- `7b7d49d5`：生产 wire 的 Value 对象递归键序规范化。根因是 render 依赖链启用 `serde_json/preserve_order`，导致 `parse_body_json` 与 `encode_cells_json` 的 Value-backed 输出键序不同。
  `Value::sort_all_objects` 只作用于 block data / encoded cells / engineInfo Value，结构体字段保留声明顺序，数组保留原顺序（marks 原有规范排序除外），未知字段和嵌套 null 不被改写。
  `engineInfo` 不伪造相同指纹；不同源码集合的元数据仍单列。
- 测试红：render 下 `wire_order` 两例均 FAILED；绿：默认/render 对同一固定 gold args 重放 **242 条 JSON/Markdown 原始 output 字符串逐字节一致**（76+77+15+9+28+11+13+13，**不包含 HTML**）。
  测试改为读已提交 gold 的 args，不再把各 feature 临时生成的 JSON 输入键序差异误算成被测函数输出漂移。没有在测试中解析/重排 output 字符串。
- `ce6cef12` Clipboard：原 S3 T3.4 经用户授权提前到 S2，实现放独立 `src/html/clipboard.rs`。完整移植旧 Dart `selectionHtml` / `_listRunHtml` / `inlineToHtml` 的块/mark 分支，复用 SpanWalker、safe_url、tagfilter、strip_unsafe_attrs，不调用数学/Mermaid rich renderer。
  保留数学源码及 data-mica 属性、脚注标记、Mermaid 代码、列表嵌套/序号/checkbox、引用分组、表格行内 HTML、图片 src 映射；HTML clipboard 原来不输出 link title，继续如此。
- 移植期望来自 WP-D `38c43744` 的真实旧 Dart 输出（`tests/fixtures/clipboard_html_port.json` 原样复制）；不是用 Rust 自生成期望。
  接线前 5 组测试全部红；接线后 default/render 各 7 组通过，其中一组逐例比较 **30 个完整全选样本原始 HTML 字节**。
  `33-tilde-fallback.md` 唯一初次差异是单个空 code 节点：旧 Dart 的全选是折叠 caret，没有 textual slice，因此输出空；代码与专项注释只匹配该选择边界，多块中的空 code 仍保留 `<pre>`。
- 安全差异（刻意保留）：旧 Dart 原样输出危险 href/src；新 Clipboard 与 Export 一样过滤 `javascript:` / tab-split scheme / `vbscript:` / 非 image data URL。专项测试按安全空 URL 断言，未为追求旧字节相同放松 URL 防线。30 例安全输入无此差异。
- 同范围 marks：重用 HTML walker 的“后出现者为外层”规则；Dart 30 例（含 `***both at once***`）及 bold/italic/strike、linked code 专项逐字节通过。wire marks 数组排序语义未改。

#### 默认 gold 变更与 Export 契约申请

默认 gold 重新生成一次，仅 `render_html_body.json` 中 Clipboard 的 **66/77** 个 output 变化；其余 8 个 JSON 文件不变。
逐例比较原 `7a9ee622` 的 154 个 HTML gold：args/id 全部不变，**Export 的 77/77 个 output 字节原样保留**。
默认与 render 下 Clipboard 都必须真执行 byte 断言，不再 cfg 排除；Export rich-feature 的差异则明确单列，不能报“全十函数跨端逐字节一致”。

冻结契约冲突：02 §2 明确 Export 继承 rich renderer（数学/Mermaid 随 feature 改变），§3.1 同时要求两个 binding 所有输出逐字节相同；桌面开启 render、wasm 不开启，两条无法同时成立。
候选最小裁决：跨端十函数一致性对 `render_html_body_json` 仅要求 **Clipboard**，Export 保留既有 feature-dependent 定义并按各自语义验收。没有自行改契约或关闭 desktop render / 给 wasm 加 render。
`wire_gold` 在 render 构建下继续验证 Clipboard 原字节，Export/no-render gold 比较显式排除并注释此冲突；这不代表 Export parity 通过。

日志（本机临时文件，未提交）：`C:/Users/willz/AppData/Local/Temp/wpr-wire-order-red.log`、`wpr-wire-json-green-{default,render}.log`、`wpr-clipboard-red.log`、`wpr-clipboard-green{,-render}.log`。

阶段验证（测试编写期间的快照，不冒称同一最终树）：`cargo test -p mica-md` 77 passed（Clipboard 7 组）；`--features render` 77 passed（Clipboard 8 组，Export/no-render replay 少 1 组）；`--no-default-features` 78 passed（Clipboard 9 组，Uuid 比较少 1 组），均无 failed/ignored。
最终 Clipboard 专项 `cargo test -p mica-md --test clipboard_html` 默认/render **各 9 passed、0 failed、0 ignored**，包含显式 Export 特性差异断言（默认 span、render SVG）与空 code 选择边界。
`cargo clippy -p mica-md --all-targets --features render -- -D warnings`（1.04s）及 `--no-default-features`（15.68s）均 exit 0。
日志：`wpr-clipboard-final-{default,render}.log`、`wpr-clipboard-final-clippy-{render,no-default}.log`。
既有门面 `cargo test -p mica-markdown`：**185 passed、0 failed、0 ignored**（38 个 target 结果汇总，含原 CommonMark/GFM/golden/conformance 测试，源码未改），日志 `wpr-clipboard-facade-tests.log`。

#### 87 字符 native release 窄探针（证据，不插入优化）

新增 `crates/md-core/examples/boundary87.rs`，输入精确使用 S0 `hotPath87`：
``Hot paths: **bold** and `code` plus a [link](https://example.com) in one paragraph now!``（ASCII/UTF-16 都是 87）。
分开测 `parse`（顺序 id / root）、`parse_body`、`wire::parse_body_json`，每次 `black_box` 消费实际返回值、包含其销毁成本，不缓存。
自适应增大 batch 到每批至少 50 ms、2 批预热、7 批取中位数；每项独立执行，不用相减解释跨层开销。
命令 `cargo run -p mica-md --release --features render --example boundary87`；rustc 1.98.0 / cargo 1.98.0 / x86_64-pc-windows-msvc。
根 workspace 与 FFI 独立 workspace 均没有 `[profile.release]` 或附加 cargo config，本进程无 RUSTFLAGS / CARGO_PROFILE_RELEASE_* 覆盖，
因此使用相同 Cargo 默认 release：opt-level=3、lto=false、codegen-units=16、debug=false、incremental=false、panic=unwind。
只与 D 的 release wire+FFI 绝对数并列，不能直接从两个独立 probe 相减归因为解析/桥接。
源码改动会触发 fresh 产物要求，example 不在核心源码指纹 ROOTS，不改变稳定 `ce6cef12` 的源码哈希。
实测源版本 `ce6cef12556f28d67828142ca96e89bf67012841`（生产源码），feature=`uuid`（默认）+`render`。Cargo 本次实际重编译 `mica-md` 并运行 `target/release/examples/boundary87.exe`（release 构建 9m46s，exit 0）；非旧 DLL 或 debug 产物。

| 独立测项 | batch | 7 批中位绝对 ms/次 |
| --- | ---: | ---: |
| parse-only（顺序 id） | 8192 | 0.009060510 |
| parse_body | 8192 | 0.009426453 |
| wire::parse_body_json | 8192 | 0.010568604 |

D 此前的 release wire+FFI 范围绝对数 0.015718 ms/次来自独立 probe/旧时点，只作上下文并列，不能与本表相减，也不是 fresh 联合预算验收。此处没有据此添加缓存、改预算或扩大优化；D/W 需在集成新源码后重建正式产物复测。
日志 `C:/Users/willz/AppData/Local/Temp/wpr-boundary87-release.log`。结构门禁 `bash scripts/check-file-size.sh` exit 0、`file-size gate: passed`（只有存量 WARN，无新增超限单元）。
本轮余下未验证项：正式 FFI/wasm 的 fresh 全路径预算、联合集成门禁及 Export 契约申请由协调者继续；不把本地纯 Rust 探针报为全平台通过。

### 2026-10-27 WP-D：最终桌面验收（集成头 + R 安全层）

执行于含 WP-R `492a9a42`（`sanitize_html` 融合扫描）、`4261970d`（stage87 精确输入）、`8a7856b8` 的集成树，D 分支 HEAD `3ca14d10`。native DLL 与 release bundle 均由该树重建；探针引擎指纹 **`ac687532799855bc`**，VM 契约同轮重建后 **42 passed / 0 failed / 0 skipped**（10 组 wire gold、30 份旧 Dart Clipboard 逐字节 port、指纹幂等回归、wrapper）。

口径说明：init 计时为**预热后**口径——每个新复制目录先做一次不计时的预热启动（Windows Defender/智能扫描对新复制二进制的首次加载耗时数秒，属复制成本，不属于引擎），随后同目录正式计时；等价于固定安装目录的第二次启动状态。预热使用各自独立的隔离 APPDATA，不共享用户数据。未预热的原始五冷（3490/13991/9048/8598/7381 ms）保留在 `wp-d-release-summary.json` 的历史运行中作为方法学诊断。

| 验收项 | 结果 | 预算/判定 |
| --- | --- | --- |
| init p50（预热口径，n=5） | **2 ms**（6/6/2/2/2；startup_chain 125–136 ms 另列诊断） | ≤30 ms ✅ |
| Unavailable（复制 bundle 移除 DLL，无 fallback） | 真实 `UnavailableMarkdownCodec`、crash.log 记录初始化错误、首帧仍渲染，init 6 ms | ✅ |
| AOT parse.87 | **0.873×**（codec 0.01728 ms / Dart 0.01973 ms） | ≤1.2 ✅ |
| AOT parse.10k | **0.640×**（3.67 / 5.78 ms） | ✅ |
| AOT parse.100k | **0.616×**（44.07 / 73.45 ms） | ✅ |
| AOT serialize.10k | **1.534×**（2.238 / 1.459 ms） | ❌ |
| AOT html.10k（clipboard） | **3.712×**（2.656 / 0.715 ms） | ❌ |
| 边界 inline87 | 0.0124 ms | ≤0.1 ms ✅ |
| VM 契约（同轮） | 42 / 0 / 0 | ✅ |

**失败项归因（与 WP-R 同输入 wire 数据对照）**

> **更正（M2 复核后）**：此前记录的“两项目标均在 FFI 边界、Rust 本体远快于 Dart”说法不成立，原因是把 **native release** 的 wire 数字与 **VM（JIT）** 的 Dart 基线相比。以同轮 AOT 的 Dart 基线重算如下。

- serialize 1.534×：Rust `wire::serialize_body_json` 同输入 **1.449 ms**，同轮 AOT Dart 序列化 **1.459 ms**，即 **≈1.0× 持平**（不是 0.63×；0.63× 是与 VM 的 2.30 ms 相比得出的，两套口径不可混用）。门面总耗时 2.238 ms = wire 1.449 + Dart `encodeBlocks` 0.42 + FFI/FRB 边界约 0.37 ms。**超标主因是边界成本叠加在“与 Dart 持平”的引擎之上**，不是引擎回退。
- html 3.712×：Rust Clipboard wire render 同输入 **1.542 ms**，同轮 AOT Dart `selectionHtml` **0.715 ms**，**2.16×**；即使只算 render 本体（278 块约 0.99 ms）也约 1.38×。门面总 2.656 ms = wire 1.542 + `encodeBlocks` 0.42 + 边界约 0.69 ms。**这一项的主因是 Rust Clipboard 渲染新增了 Dart 旧实现从不做的安全过滤**（`sanitize_html`：tagfilter、危险属性与 URL 剥离），边界是次要增量，不能归为纯边界问题。审核方在 Linux 上的同量级探针（1 万字符 Clipboard wire）只有 **0.387 ms**，其中 JSON 解码约 0.1 ms —— 说明引擎本体没有异常，1.542 ms 与 0.387 ms 的差别来自两处探针口径不同（平台、输入、是否含解码），未逐一对齐；两种口径都指向「渲染本体 + 新增安全过滤」，不是边界。
- 因此两项的性质不同：serialize 是边界成本（Rust 本体已与 Dart 持平），html 是渲染本体 + 边界共同导致。缩小边界需要批量/二进制传输，会改变冻结的 wire 契约形状；缩小渲染本体需要继续优化 Rust Clipboard 路径。两项都作为**待审核裁决项**提交，执行方未擅自改契约或放宽预算。数值来自同轮 AOT（Windows release，`flutter build windows --release -t integration_test/markdown_codec_release_probe.dart` + 同进程 Dart 对照），未与 VM JIT 或 Chrome 数混算。

**证据**：`clients/mica_flutter/build/wp-d-release-summary.json`（五冷、bench、Unavailable、指纹）、每次运行的 `build/wp-d-release-*/run-N/driver.json`（含 console 与清理状态）、`build/wp-d-vm-codec-bench.json`（JIT 诊断）、`build/wp-d-dart-boundary-split.json`（Dart 边界分解）。探针驱动只操作复制 bundle、自有 PID 与自建隔离 APPDATA，并在 finally 中有界终止/清理；未触碰用户安装或用户数据。

**M2 桌面侧材料**：通过 7 项（init、Unavailable、parse×3、边界、VM 契约），失败 2 项（serialize、html，均含完整归因与 Rust 侧已达证据），连同 Export profile 跨 feature 字节一致性问题一并交审核方裁决。

#### Clipboard 与 wire 最终核心接入（2026-10-27）

WP-R `7b7d49d5`（输出 Value 递归稳定键序）接入为 `57bb4942`，`ce6cef12`（提前 T3.4 Clipboard 独立于 render）接入为 `62f4d05e`；未在 WP-W 手改核心。官方公共门面基准脚本已提交 `44cb7c0a`。R 后续 `e5ae374d` 只改 native probe/example 与记录，不在 wasm 指纹输入中，不为此额外重建。

最终正式构建实际 exit0：先执行无默认特性 checked-in gold **9 passed / receipt 9 (floor 9)**，再按 opt3 / LTO / 全依赖 codegen-units=1 / stable / --locked 构建。分文件 gzip -n -9 得 **201363 B wasm + 2744 B JS = 204107 B**，低于 204800 B 硬门槛，余量 **693 B**；没有删除 API、添加诊断导出或放宽门槛。WASM SHA-256 **`df8e8e5af058c503dfe118ec2cde67c1f9c7c15ea51782d2928671a269153aff`**，JS SHA 不变 `90ee81fdb2466b0e2a497216203aa27907cc42777997f27b55b6141634d9a532`，源码指纹 **`562668d7bee36096`**。仅保留十个业务导出和工具支撑；产物与 test/support 的九份 gold 复制 diff exit0。

`node scripts/test-md-wasm-gold.cjs` 对最终 WASM 当前黄金 **9 文件 / 396 cases** 字节全同，含更新后的 Clipboard；engineInfo 当前源集指纹单独验证、六类 Error 与后续恢复通过。HTML 文件当前 Export/Clipboard 各 77 cases；Node 只证明实现匹配当前 gold，不独立证明黄金历史未改。Export 原样的历史结论来自 WP-R 交付验证，WP-W 的额外 Node child_process/git before-after helper被隔离检查拒绝，未执行且未换入口绕过。

最终核心下 `cargo test --locked -p mica-md-wasm` forward **10 passed / 0 failed / 0 ignored**；`cargo clippy --locked -p mica-md-wasm --all-targets -- -D warnings`、根 fmt、shellcheck、构建行为 fixture 七项、ci-paths、structure、ignored-source 均 exit0。日志位于 `023fa81a:scripts/md-wasm-evidence/md-final-*.txt`。

#### T2.13 最终真实公共门面测量（Clipboard / wire 修复后）

**正式 Flutter target 编译实际 exit0**：`flutter build web --release --no-web-resources-cdn --target=../../scripts/md-wasm-bench.dart --output=build/md_wasm_bench`，日志 `023fa81a:scripts/md-wasm-evidence/md-final-facade-web-build.txt`。正式业务路径为 **`MarkdownCodec.parseBlocks -> _WasmMarkdownCodec -> markdown_wire.decodeBlocks`**，初始化经过公共 `initMarkdownCodec()`；raw 同时记录真实 facade_engine，指纹为 **`562668d7bee36096`**。旧手工 `_full` wire 测量不作为此次门面验收数据。

同一产物清单：WASM SHA **`df8e8e5af058c503dfe118ec2cde67c1f9c7c15ea51782d2928671a269153aff`**，JS SHA **`90ee81fdb2466b0e2a497216203aa27907cc42777997f27b55b6141634d9a532`**，Dart release `main.dart.js` SHA **`f986f14193ec2dac11098e670040ddea22c64199e79e9ad2645970638dca1399`**。服务目录文件已与正式产物哈希核对；WASM 532652 B/JS 15310 B 为未压缩大小，预算仍取分别 gzip 后 **204107 B**，余 **693 B**，不能混用。

原有 exact S0 三档不变；Chrome 156、Windows 11、真实 release dart2js；同页现有 markdownToBlocks 对照，完整公共门面同步调用、JSON 往返与解码全部计时。每档三轮，每轮预热后 15 批 AB/BA（批次 1024 / 8 / 1）；先算每轮两路径中位数之比，再取三轮比值中位数。

| exact S0 输入 | 三轮实际门面 / Dart | 比值中位数 | 实际门面每轮 p50 ms |
| --- | --- | ---: | --- |
| 87 UTF-16 | 1.088050 / 1.051948 / 1.073333 | **1.073333** | 0.016895 / 0.015820 / 0.015723 |
| 10000 UTF-16 | 0.523207 / 0.505176 / 0.533186 | **0.523207** | 3.1000 / 3.0500 / 3.0125 |
| 100000 UTF-16 | 0.572347 / 0.560062 / 0.569355 | **0.569355** | 35.600 / 35.900 / 35.300 |

三档**每轮均 ≤1.2**；短档最慢 **1.088050**，没有使用旧手工 wire 较快数字替代真实门面。raw 的原始未按 mark 顺序规范化 `equivalent` 值仍保留，长档顺序差异解释沿用上文契约 §3.4；此次未再扩输入矩阵。

五个全新 browser context 的初始化 **13.5 / 26.5 / 12.9 / 14.0 / 14.5 ms**，p50 **14.0 ms**、最大 **26.5 ms**，均低于 60ms。计时边界为生产 `micaMdReady` Promise **赋值到完成**：在页面脚本执行前定义 setter，附 `.then` 在完成时记录 `performance.now()`，**包含 wasm fetch、编译/实例化及 Promise 完成**，不包含更早的 JS 胶水下载；另记录 instantiateStreaming 起点用于区分（4.8/17.9/4.4/4.5/5.0 ms）。这是同一浏览器进程内五个新 context，可复用进程编译缓存，不代表冷机器/公网网络。未使用 waitFor 轮询时间作为 resolve 时间。服务器 no-store，不提供 COOP/COEP，crossOriginIsolated=false。

六类畸形 JSON/非法 profile 直接调用正式 JS API 均抛 Error，随后同实例 parseBody 正常；该错误测试为 WASM 绑定边界，不宣称测试了所有 Dart 门面异常包装（由 WP-D 负责）。初始化、比值原始 15×3 样本、浏览器 UA/visibility、错误恢复、官方门面 path/facade_engine、三个文件 SHA、build 参数全部关联在 **`023fa81a:scripts/md-wasm-evidence/md-m2-official-facade-final.json`**。截图 `target/md-m2-official-facade-final.png`。未新增重测矩阵；测量结束已停止自有静态 server 并关闭 Playwright 页面，未触碰 D 的 Windows 进程。

本次 WP-W 证据支持正式十 API 产物 **体积、WASM 初始化、Chrome 实际公共门面三档解析预算达标**，最终 M2 结论留给主会话审核读取证据后判定。**Export render 例外仍单列**：无 render WASM 的 Export 黄金通过，仅证明当前 WASM 匹配当前黄金，不证明富渲染 FFI Export 与 WASM 逐字节一致，不宣称跨端 all-ten-byte 全绿。新版 wire_gold 已恢复 render 下 Clipboard 黄金，只有 Export 固定输入例外（不再整项 HTML cfg 排除）。桌面三平台契约/Windows runner、首帧推迟、桌面预算和安装包由 WP-D/集成方提供实际回执；WP-W 没有重复 Windows 重型构建。T2.14 名单待 D 实际通过后由主会话在已有测试文件的集成树登记。

### M2 复核实测：Linux 正式构建与 Chrome 复制预算（2026-10-27，WP-W）

对应审核方提出的四条中的前两条（门槛 262,144 B 已由 `d1623da9` 改入 `scripts/build-md-wasm.sh`）。

#### ① Linux 正式十 API 构建

| 项 | 结果 |
| --- | --- |
| 树 | `single-source@9e604c43` 完整 `git archive` 导出（九个 workspace 成员齐全），行尾按提交原样 LF |
| 环境 | Docker `rust:1.98-slim-bookworm`（Debian 12）；镜像默认 rustc 1.98.1，`rust-toolchain.toml` 钉的 1.98.0 由 rustup 自动安装并主导构建；`wasm-bindgen-cli` 0.2.129 与 `Cargo.lock` 一致（`check-md-wasm-toolchain.sh` 硬校验） |
| 参数 | `--locked`、wasm 包 `opt-level=3`（根 `Cargo.toml` 的 `[profile.release.package.mica-md-wasm]`）、`lto=true`、全依赖 `codegen-units=1`、`panic=abort`（wasm32 默认即是）、`--remove-name-section --remove-producers-section` |
| 体积（分文件 `gzip -n -9` 后相加） | **wasm 202,525 B + JS 2,744 B = 205,269 B**，≤ 262,144 B（余量 56,875 B） |
| wasm SHA-256 | `d1bdfd3d9c2bf8444da2665472cf0146e94dc936853b92081628a30be17120a3` |
| JS SHA-256 | `90ee81fdb2466b0e2a497216203aa27907cc42777997f27b55b6141634d9a532`（与 Windows 产物**逐字节相同**） |
| 源码指纹 | `12679a6fb723da2a`（Windows 为 `562668d7bee36096`） |

指纹差异说明：两平台 ROOTS 源集相同，差异来自构建输入是**导出树**还是**工作树检出**（路径上下文参与指纹），不是内容差异；JS glue 逐字节相同可佐证两平台使用的是同一 wasm-bindgen 版本与同一导出面。未改 API、未加诊断导出、未放宽任何预算。

**跨平台对照**：Windows 204,107 B → Linux 205,269 B，差 **+1,162 B**（同一 wasm 源码在两种工具链下的压缩差异）；两平台均在新门槛 262,144 B 内。

**复算**：主会话对上述产物独立执行 `sha256sum` 与 `gzip -n -9`，wasm/JS 两个 SHA 与体积全部一致，非仅采信执行方口述。

**复现脚本**：本次容器内实际使用的 `scripts/m2-linux-run.sh`（安装 pin 住的 CLI 后调用构建）与 `scripts/m2-linux-build.sh`（`--locked` 十 API 构建 + wasm-bindgen + 指纹 + 体积）随本记录一并提交，审核方在 Linux 上可直接用同一组命令重建并与上表比对。
> **审核方注（M2 复核）**：这两个脚本与 `scripts/build-md-wasm.sh` 功能重复、写死容器路径，合并前已删除（可在 `e7b5cb10` 取回）；Linux 复现直接跑 `bash scripts/build-md-wasm.sh`。

#### ② Chrome 生产构建 Web 复制预算

| 项 | 结果 |
| --- | --- |
| 产物 | 同一 production `dart2js` 页面：wasm `df8e8e5a…`（= Windows 正式产物）、glue `90ee81fd…`、`main.dart.js` `1869955a…` |
| 路径 | 真实公共门面一次复制：`MarkdownCodec.serializeBlocks` + `renderClipboardHtml` |
| 输入 | 10,000 UTF-16 / 298 块；每轮预热 3 次后计时 15 次，**共三轮独立测量**（每轮新浏览器 context 重新加载同一产物） |
| 结果 | 三轮中位 **4.6 / 4.3 / 4.1 ms**，汇总中位 **4.3 ms ≤ 16 ms**（样本范围 3.7–5.9 ms）；产物 10,480 B Markdown + 15,331 B 剪贴板 HTML |
| 口径 | Chrome 156 / Windows 11 / 生产 dart2js；不是桌面或原生的数字 |

测量脚本为 `scripts/md-wasm-bench.dart` 的 `copy.10k` 场景（随本记录提交，函数 `_measureCopy`），因此在同一正式构建上可重放。测量结束已停止自有静态服务并关闭浏览器页面，未触碰用户进程。

**证据存放说明**：本轮原始 JSON（`md-m2-linux-build.json`、`md-m2-copy-web.json`、三轮版 `md-m2-copy-web-3rounds.json`）与两份 Linux 产物留在 WP-W 工作树 `scripts/md-wasm-evidence/`，**不随本记录再提交**——与 `b0064835` 删除证据目录的审核要求一致。数字可由同一提交、同一参数重建复算：体积与 SHA 用上文命令直接比对，JS glue 的逐字节相同可先于完整重建核对。

