# 网页端首次打开白屏（2026-11-16）

## 现象

无痕窗口打开 `https://mica.cloudcele.com/`，最初几秒整页空白；第二次打开稍快。

## 结论

- **不是思源字体造成的**。网页端不下载 8.5 MB 的桌面字体包（那是 Windows 桌面专用），中文走引擎按需加载的同源字体切片，每片约 30 KB，而且在首帧之后才开始加载。`warmUpFonts()` 在网页端只做一次排版，不发请求。
- **首帧前必须先下完约 4 MB（gzip 后）**：
  - `canvaskit.wasm` 约 2.0 MB；
  - `main.dart.js` 约 1.45 MB；
  - Markdown 与文档两个引擎的 wasm 约 0.5 MB；
  - 字体清单与三个默认字体。
  下完之后还要编译、执行约 0.5 秒。
- **加载是串行的**：`index.html` → 引擎 glue → `flutter_bootstrap.js` → `canvaskit` 与 `main.dart.js` → `FontManifest.json` → 字体，每一步都要等上一步到达。
- 这段时间页面上没有任何内容，浏览器连首次绘制都没有发生，所以是纯白屏。

## 改动

1. **启动画面**：`web/index.html` 内联 Mica 标志，HTML 一到就显示，Flutter 首帧（`flutter-first-frame` 事件）时移除。300 ms 后才淡入，热启动不会闪一下。标志用内联 SVG 而不是 `<img>`：图片请求会排在大文件预加载后面，等于没有。
2. **预加载**：同一个内联脚本在解析 HTML 时就为首帧需要的所有文件插入 `preload` / `modulepreload`：
   - CanvasKit 按 flutter.js 的同一判断选 Chromium 版或完整版，避免 Firefox、Safari 白下 2 MB；
   - `fetch` 类预加载带 `crossorigin`，否则浏览器会下载两次。
   两个引擎的 `<script>` 挪到 body 里启动画面之后，浏览器等它们时可以先画出启动画面。
3. **Brotli**：`deploy/Dockerfile.web` 构建镜像时为每个 JS / wasm 文件生成 `.br`，`deploy/nginx.conf` 的 `\.(js|wasm)$` 块对声明支持 `br` 的客户端改发 `.br` 文件，其余照旧 gzip。
   - 官方 nginx 镜像没有 Brotli 模块，所以用预压缩文件而不是实时压缩；不碰共享 Traefik。
   - 大文件体积：canvaskit 2033 → 1505 KB，main.dart.js 1452 → 1098 KB，两个引擎 wasm 各约 250 → 190 KB。
   - 这些固定文件名的 JS / wasm（包括 `canvaskit/`）统一 `Cache-Control: no-cache`，靠 ETag 返回 304。`canvaskit/` 不带内容哈希，Flutter 升级后同名替换，原来的启发式缓存可能把旧 canvaskit 配上新 `main.dart.js`。
4. **CSP 哈希守护**：新增内联脚本后同步更新两份 nginx 配置里的 `script-src` 哈希；`scripts/test-web-csp-hashes.sh` 校验两边一致（缺的、多余的都报错），已接入 CI 和本地 CI。

## 测量

本地静态服务，同一份 release 构建，Playwright 冷启动，CDP 限速，各取两到三次：

| 网络 | 改前首帧 | 只加预加载和启动画面 | 再加 Brotli（真实 nginx 配置） |
| --- | --- | --- | --- |
| 120 ms 延迟 / 25 Mbps | 约 2.55 s | 约 2.25 s | **约 1.85 s** |
| 300 ms 延迟 / 10 Mbps | 约 5.07 s | 约 4.30 s | **约 3.53 s** |

- 改前首帧之前一直是白屏；改后 HTML 到达后约 300 ms 显示标志（300 ms 延迟的网络下第 1 秒截图里已有标志）。
- Chrome 的 FCP 指标不把淡入中的启动画面计为"内容绘制"，所以表里看首帧时间，启动画面用截图确认。

## 还可以做但这次没做

- **缩小 `main.dart.js`**（5.2 MB 未压缩）：把设置、导入、图谱等首屏用不到的部分改成延迟加载库。收益可观，但要改代码结构。
- **改用 skwasm 渲染**（`flutter build web --wasm`）：在支持 WasmGC 的浏览器上更快，需要单独评估兼容性和体积。
