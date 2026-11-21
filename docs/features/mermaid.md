# Mermaid 图表渲染

> 状态：**已实现并三端统一（纯 Rust `merman` 引擎）** · 更新日期：2026-11-21

## 1. 架构定位与选型

Mica **不使用** JavaScript 版 [`mermaid-js/mermaid`](https://github.com/mermaid-js/mermaid)（无浏览器 DOM 依赖、无 Node.js、无 `mermaid.min.js`）。所有平台的 ```` ```mermaid ```` 代码块统一由**纯 Rust 引擎 [`merman`](https://crates.io/crates/merman)（`0.7`，`render` 特性）**渲染为 SVG：

- **数据模型不变**：Mermaid 在文档模型里仍是普通 `code_block`（`data.language == "mermaid"`），Markdown round-trip 字节保持原样；光标移入块内编辑源码，移出后展示渲染结果（亦支持在预览/源码之间切换）。
- **单一真相源**：服务端 HTML 导出/公开分享、桌面/移动端编辑器、Web 端编辑器全部调用 `crates/md-core` 中同一份 `render_mermaid_svg_with_id` / `render_mermaid_svg` 入口，避免三端排版与语法解析分叉。

### 为什么不用 `mermaid-js/mermaid`

| 维度 | `mermaid-js/mermaid`（已弃用） | 纯 Rust `merman`（现方案） |
| --- | --- | --- |
| 运行时依赖 | 强依赖浏览器 DOM 测字与布局 | 零 JS / 零浏览器依赖，可在服务端、桌面 FFI、WASM 同构运行 |
| HTML 导出 / 分享页 | 服务端无法直接离线出图 | Rust 原生直接内联 `<div class="mermaid"><svg>…</svg></div>` |
| 平台一致性 | Web（JS）与桌面（Rust）容易出现渲染差异 | 服务端、桌面、Web 三端同一引擎 |
| 已知取舍 | 语法覆盖最全，但仅限浏览器 | 极少数冷门/最新 Mermaid 语法可能不支持（失败时安全降级显示源码）；Web 按需 WASM 包约 1.90 MB gzip（普通页面零下载） |

---

## 2. 三端调用链路

```text
                       crates/md-core (feature = "render-mermaid")
                       render_mermaid_svg_with_id / render_mermaid_svg
                       merman::render::HeadlessRenderer + SvgPipeline::resvg_safe()
                                 │
          ┌──────────────────────┼──────────────────────┐
          ▼                      ▼                      ▼
   服务端 / 导出 / 分享        桌面 / 移动端 (FFI)       Web 端 (Web Worker + WASM)
   crates/md-core/src/      clients/mica_flutter/    crates/md-wasm/src/preview.rs
   html/html.rs             rust/src/api/render.rs   (feature = "preview-mermaid")
   直接内联 SVG               render_mermaid_svg()          │
                                 │                   web/mica_preview_worker.js
                                 │                   按需加载 /mica_preview/mermaid.{js,wasm}
                                 └──────────┬───────────────┘
                                            ▼
                         clients/mica_flutter/lib/editor/mermaid_preview.dart
                         1. mermaidThemeFor(MicaTokens) → MermaidHostTheme
                         2. inlineMermaidCss(raw, background) 拍平 <style> + 合成箭头
                         3. rasterizeSvg(svg, targetWidth * 2) → flutter_svg 光栅化为 ui.Image
                            （CJK 字体映射 + 单边上限 4096px 保护）
```

### 2.1 Rust 核心层

- **依赖声明**：[crates/md-core/Cargo.toml](../../crates/md-core/Cargo.toml) 定义可选特性 `render-mermaid = ["dep:merman"]`（由 `render` 特性汇总开启）。
- **核心渲染**：[crates/md-core/src/render.rs](../../crates/md-core/src/render.rs)
  - `MermaidHostTheme`：接收宿主调色板（`dark`、`canvas`、`surface`、`surface_alt`、`text`、`subtle_text`、`border`、`line`、`error`、`warning`、`success`），构造 `HostThemeProfile` 传给 `HeadlessRenderer::new().with_strict_parsing().with_diagram_id(id)`。
  - 使用 `SvgPipeline::resvg_safe()` 输出不依赖浏览器 CSS/HTML `<foreignObject>` 的安全 SVG。
  - 解析或渲染失败返回 `None`（不 panic，不阻塞页面）。
- **HTML 导出与公开分享**：[crates/md-core/src/html/html.rs](../../crates/md-core/src/html/html.rs)
  - 遇到 `language == "mermaid"` 代码块时以块 ID 派生的 `mermaid_id(&block.id)` 调用 `render_mermaid_svg_with_id`（`theme: None`，使用 merman 默认浅色导出主题）。
  - 渲染成功输出 `<div class="mermaid"><svg>…</svg></div>`；失败则回退为原始 `<pre><code class="language-mermaid">`。

### 2.2 桌面与移动端（FFI）

- [clients/mica_flutter/rust/Cargo.toml](../../clients/mica_flutter/rust/Cargo.toml) 开启 `mica-md` 的 `render` 特性，由 [clients/mica_flutter/rust/src/api/render.rs](../../clients/mica_flutter/rust/src/api/render.rs) 暴露 `render_mermaid_svg(source, theme)` 给 Dart（[preview_engine_io.dart](../../clients/mica_flutter/lib/editor/preview_engine_io.dart)）。
- 桌面与移动端完全离线可用，不再依赖任何独立 Dart `merman` 包或额外 `merman_ffi.dll`。

### 2.3 Web 端（按需 WASM + Web Worker）

- **构建与体积预算**：[scripts/build-preview-wasm.sh](../../scripts/build-preview-wasm.sh) 单独构建 `crates/md-wasm` 的 `preview-mermaid` 特性（`opt-level="z"` + LTO + `panic=abort`），产物输出到 `clients/mica_flutter/web/mica_preview/mermaid.{js,wasm}`，硬预算 **2 MiB**（`gzip -n -9` 实测约 1.90 MB），与 256 KiB 启动包完全隔离。
- **按需加载与隔离**：[clients/mica_flutter/web/mica_preview_worker.js](../../clients/mica_flutter/web/mica_preview_worker.js) 仅在文档首次请求 Mermaid 预览时于独立 Worker 内 `import('./mica_preview/mermaid.js')`；普通不含 Mermaid 的页面零下载，且解析不受信任文档不会卡住 UI 主线程。
- **失败分级与 Worker 自愈**（与 [render-architecture.md](../architecture/render-architecture.md) Decision 2 一致）：
  - **源码无法渲染**：返回 `[id, null]`，该块保留可编辑源码，不再重复请求。
  - **渲染器不可用（`unavailable` / `PreviewUnavailable`）**：如网络失败或文件缺失，Worker 自行 `close()`（避免浏览器在同 Worker 内缓存失败的 ES module import），宿主 [preview_engine_web.dart](../../clients/mica_flutter/lib/editor/preview_engine_web.dart) 与 `RasterPreviewPipeline` 以 1s→30s 指数退避重建 Worker 重试。
  - **单条源码触发 WASM panic（`crashed`）**：Worker 立即退休，仅将肇事源码标记为失败，排队中的其他预览由新 Worker 重试。

---

## 3. 主题映射、CSS 内联与 SVG 光栅化

编辑器画布在桌面、移动与 Web 三端共用同一套 Dart 后处理管线（[mermaid_preview.dart](../../clients/mica_flutter/lib/editor/mermaid_preview.dart)）：

1. **宿主主题映射** — [mermaid_theme.dart](../../clients/mica_flutter/lib/editor/mermaid_theme.dart)
   - `mermaidThemeFor(MicaTokens)` 将当前浅/深色语义色转换为 `#RRGGBB`（去除 alpha，防止节点半透明透出底色）传入 `merman`。
   - 预览缓存 key 包含当前主题调色板，切换浅/深色时自动失效重绘。
2. **SVG CSS 展开与箭头合成** — [mermaid_svg_inline.dart](../../clients/mica_flutter/lib/editor/mermaid_svg_inline.dart)
   - `flutter_svg` 不解析 `<style>` 中的后代选择器，也不渲染 `<marker>` 箭头。
   - `inlineMermaidCss` 按选择器优先级把 `<style>` 展开到各元素的行内 `style` 属性，注入画布背景色，规范化 `font-weight`（`bolder`/`lighter` → `bold`/`normal`），并解析 `<path>`/`<line>` 端点切线方向将 `<marker>` 几何体合成为真实 `<g>` 箭头。
   - 这是 `merman` 文档定义的宿主集成边界；若 XML 解析异常则原样返回 SVG。
3. **CJK 字体与尺寸上限光栅化** — [svg_raster.dart](../../clients/mica_flutter/lib/editor/svg_raster.dart)
   - 含 CJK/韩文标签的 `<text>`/`<tspan>` 注入 `svgCjkFontFamily(...)`，避免出现豆腐块。
   - 按 `targetWidth * 2` 缩放光栅化为 `ui.Image`，缩放倍率上限 `8.0`、单边像素上限 `4096`，防止超长时序图分配过大纹理内存。
4. **画布块渲染** — [block_renderers.dart](../../clients/mica_flutter/lib/editor/block_renderers.dart)
   - 通过 `AtomicBlockRenderer` 注册表挂接 `MermaidRenderer`；仅当 `language == 'mermaid'` 且光标不在块内（或未切回源码态）时绘制栅格预览，支持拖拽平移；预览未就绪或渲染失败时返回 `null` 自动回退为语法高亮代码块。

---

## 4. 验证与回归门禁

- **Rust 导出测试**：[crates/markdown/tests/render_export.rs](../../crates/markdown/tests/render_export.rs)（正常 Mermaid 转内联 SVG、非法源码回退代码块）、[crates/md-core/tests/public_api.rs](../../crates/md-core/tests/public_api.rs)。
- **Dart 单元测试**：CSS 内联、箭头合成、主题映射、预览管线重试退避与桌面 FFI 渲染用例（`clients/mica_flutter/test/`）。
- **Web 端到端与发布门禁**：
  - `scripts/build-preview-wasm.sh` 强制检查 Mermaid WASM 压缩体积 ≤ 2 MiB。
  - `e2e/rich_preview_e2e.mjs` 验证 Web Worker 实际渲染 Mermaid、中文标签无方框以及拦截/恢复自动重试。
  - `scripts/web-entry-cache-check.sh` 校验 `/mica_preview/mermaid.{js,wasm}` 返回 200 及正确 MIME，并确认已退役的 `/mermaid.min.js` 返回 404。
