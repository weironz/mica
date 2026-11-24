# `nsh` 依赖治理与升级检查表（P2-2）

> **当前锁定版本**：`nsh-* = "=0.6.1"`（Apache-2.0，维护方：Longbridge）  
> **位置**：[`clients/mica_rust/Cargo.toml`](../../clients/mica_rust/Cargo.toml)  
> **隔离边界**：`nsh-*` 严格限制在 L3 表现层（`clients/mica_rust`），禁止进入 `crates/` 下任何 L1 / L2 crate。

---

## 1. 版本锁定策略

`nsh` 处于 `0.x` 快速迭代期，且 `clients/mica_rust` 依赖树超过 700 个 crate（包含 `wgpu`、`gtk4`、`gstreamer`、`naga` 等）。为防止常规 `cargo update` 静默引入破坏性 UI 改动、IME 退化或 WASM 体积膨胀：

1. `clients/mica_rust/Cargo.toml` 中所有 `nsh-core`、`nsh-base`、`nsh-platform-{web,windows,macos,linux}`、`nsh-engine`、`nsh-testing` 一律使用**精确版本约束 `=0.6.1`**。
2. 每次升版必须作为一个独立提交或 PR 完成，并逐项走完下方升级检查表。

---

## 2. `nsh` 升级检查表

### 升级前
- [ ] 通读上游 `nsh` 新版本的 Release Notes / Changelog，重点排查：`virtual_list`、`input` / IME、`overlay()`、`Canvas`、`nsh.js` 引导协议（`createPlayer`、`__nsh_imports`、`__nsh_finalize`）及 `nsh-platform-web` 默认特性变化。
- [ ] 检查 `nsh-platform-web` 是否改写了 `web/nsh.js` 中的 WASM `fetch(...)` 凭据/模式；若有变化，同步核对 `clients/mica_rust/web/index.html` 的 `<link rel="preload" as="fetch" crossorigin>`，防止浏览器重复下载 `.wasm`。

### 升级后（必过门禁）
- [ ] **编译与 lint**：`cargo clippy --manifest-path clients/mica_rust/Cargo.toml --all-targets --locked -- -D warnings` 0 警告；Linux CI（GTK4 + GStreamer）与 Windows 原生构建全绿。
- [ ] **单测与集成测试**：`cargo test --manifest-path clients/mica_rust/Cargo.toml --locked` 全绿；`crates/mica-editor` 与 `crates/mica-client` 全绿。
- [ ] **W5 首包体积门禁**：运行 `bash scripts/build-web-rust.sh`，确认 `mica_rust_bg.wasm.br + mica_rust.js.br + nsh.js.br` ≤ **4,194,304 B（4.0 MiB）**，且 `scripts/test-web-csp-hashes.sh` 通过。
- [ ] **P1-3 Web 端到端**：运行 Web Playwright E2E，确认登录、编辑、双端 WS CRDT 同步、IndexedDB 离线恢复、公式/Mermaid 懒加载与 0 CSP 违规全部通过。
- [ ] **P1-2 真人输入法抽检**：在 Windows 桌面（微软拼音 / 搜狗）及 Web（Chrome / Firefox / Safari）复核拼音组字、候选窗跟随、`Esc` 取消、`Backspace` 不误删正文与跨块选区。

---

## 3. 已登记的上游 `nsh 0.6.1` 已知问题与绕过方案

| 编号 / 现象 | 影响 | Mica 当前绕过 / 缓解 | 上游跟进点 |
| --- | --- | --- | --- |
| **`nsh::RichEditor` 缺少块级虚拟化（上游 #2376）** | 长文档一次性挂载全部节点，内存与帧率不达标 | **路线二-B**：彻底弃用 `nsh-base::RichEditor`，改由自有 `crates/mica-editor` + NSH 原生 `virtual_list` + 单块细粒度 `Signal` 驱动 | 保持路线二-B，不回退 `RichEditor` |
| **`nsh-platform-web 0.6.1` 无条件启用 `nsh-core/glsl` 及 `nsh-base` 全量特性** | 即使 `clients/mica_rust/Cargo.toml` 设为 `default-features = false, features = ["editor"]`，`nsh-platform-web` 仍强制拉入 `naga`（~0.54 MiB 代码）、`rich-editor`、`term`、`plot`、`math`、`highlight`、`emoji-picker` | 通过 **W4** 拆出 `mica_preview/{math,mermaid}` + **W2** `wasm-opt -Oz` 将首包压至 **3.74 MiB**（远低于 4.0 MiB 门禁） | 建议向上游提 PR/Issue，将 `nsh-platform-web` 对 `nsh-core/glsl` 与 `nsh-base` 扩展特性改为可选 feature，可再释放约 0.5–0.8 MiB 体积 |
| **NSH 矢量 `svg()` 渲染路径丢弃 `<text>` 节点** | Mermaid 流程图中的中文/英文节点标签不可见 | Mermaid 与白板预览统一走 `image(ImageSrc::svg_markup(...))`（`resvg` + 系统字体光栅化），并在 `block_views::tests::mermaid_cjk_svg_rasterizes_dark_text_pixels` 锁死回归 | 升级后若矢量路径支持字体排版，可再评估是否切换 |
| **子 `Owner` 销毁导致弹窗内异步任务被取消** | 关闭登录/账号/右键菜单时，挂载在局部 `when(...)` 下的 `spawn` 任务被静默中止 | 全部长生命周期异步任务（登录、登出、WS 同步、文件选择、预览懒加载）统一绑定在根 `app_owner.run(...)` 上 | 保留该架构约定，不依赖局部 UI `Owner` 存活 |
| **非 NSH 响应式运行时（如 `wasm_bindgen_futures::spawn_local`）写入 `Signal` 会 panic** | `nsh-core/src/element/reactive.rs` 报 `no reactive runtime` | Web 异步回调一律走 `nsh_core::prelude::spawn`（配合根 `app_owner`） | 保持统一封装，禁止直接用 `spawn_local` 写 `Signal` |
| **Linux 平台后端依赖 GTK4 + GStreamer** | Ubuntu CI 若仅装 `libgtk-3-dev` 会编译失败 | CI / `release.yml` 显式安装 `libgtk-4-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev pkg-config` | 随上游系统依赖同步维护 |
| **Web Firefox 兼容性 / import map 边界** | 需关注 Firefox 下 ESM 动态 `import()` 与 `<link rel="modulepreload">` 行为 | `index.html` 采用标准 `<script type="module">` + 直接相对路径动态 `import()`，不依赖 `importmap` | P1-2 真人浏览器清单持续跟踪 Firefox |
