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
