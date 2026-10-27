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

