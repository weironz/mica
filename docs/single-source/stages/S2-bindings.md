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

最终 GNU gzip `-n -9 -c` **分别**压缩后相加：wasm **196936 B** + JS **2744 B** = **199680 B（195 KiB）**；低于 204800 B 硬门槛，高于 122880 B 软目标。减小来自删除元数据与统一依赖 codegen，未删除 API、未换压缩口径。原首次失败日志在任务输出 `bk1q1kr2f.output`；正式成功及严格 gold 缺失失败日志保存在 `scripts/md-wasm-evidence/`。首次未 strip 原产物已被后续构建替换，因此不能声称保留其二进制哈希；strip 中间文件还在 `target/md-size-check/`，SHA `e792a6e7ade84d76a433f5ec816dd9b19115dd4c042287e6f98eb559fa227e91`。

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

原始数据及复核入口：`scripts/md-wasm-evidence/{md-m2-browser-final.json,md-m2-browser-raw.json,md-m2-diff-summary.json,inputs.json,html-feature-summary.json,md-runtime-errors.json}`；测量代码 `scripts/md-wasm-bench.dart`，host 复核 `crates/md-wasm/examples/wire_stdin.rs` + `scripts/md-wasm-diff.cjs`。运行时临时数据在 `target/`，截图 `target/md-m2-browser.png`。测量仅覆盖 parse Document 与 Chrome，不外推 Fragment、Firefox/Safari、序列化/HTML 或桌面性能；Firefox 补测未做。

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

本节取代上文“未接入/未重跑”的当前状态，但保留历史失败。新的直接 Cargo 命令及原始日志均在 `scripts/md-wasm-evidence/md-post-*.txt`：

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

每轮均 ≤1.2。五个新 browser context 初始化（Promise 赋值到 fulfillment，同方法）为 **35.1 / 13.7 / 11.4 / 13.3 / 12.3 ms**，p50 **13.3 ms**；crossOriginIsolated=false。六类非法 JSON/profile 在新产物浏览器均抛 Error，随后同实例 parseBody 正常。新清单、全部原始样本、初始化与异常记录在 **`scripts/md-wasm-evidence/md-m2-post-integration.json`**；旧 browser-final/raw 文件未覆盖。临时截图 `target/md-m2-post-integration.png`；静态服务器测完已停止。

新增 `node scripts/test-md-wasm-gold.cjs` 直接加载正式 wasm，对 checked-in 默认黄金的返回字符串逐字节重放：parse_body **76**、parse_inline_batch **15**、decode_cells **28**、encode_cells **11**、serialize_body **77**、serialize_inline **9**、render_html_body **154**、math_run_spans **13**、parse_inline_math **13**，合计 **9 文件 / 396 cases** 全部通过。engineInfo 单独断言 schema_version 与该 WASM SOURCE_FINGERPRINT 一致（不与核心不同源集指纹强行相等）；六类错误+随后恢复也通过。该 Node 专项验证**不能替代 Dart VM/web/Windows 契约，也不能证明实际 FFI render 字节相同**。

当前剩余阻塞不变：render HTML 因 `cfg` 不在 wire_gold render 测试里执行，默认 9 绿不能称真实 FFI 全九项绿；固定默认 args 的 HTML 实际内容差异、parse_body/encode_cells 对象键序差异继续待审核裁决。未改测试去掩盖这些失败。WP-D 门面与 Windows 实际集成/首帧/安装包验证仍不在本工作包完成证明中。

