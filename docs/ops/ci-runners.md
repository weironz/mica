# CI 与 runner

GitHub Actions 使用官方 runner（`ubuntu-latest`、`windows-latest`、`macos-latest`），
所有入口保持手动触发。自建 runner 已停用；专用 bootstrap/probe 工作流与 Flutter 预装复用逻辑已删除。
Flutter SDK 由统一 action 按 `.fvmrc` 下载并缓存。

## 两条门禁路径

- Actions 可用时，手动触发 `CI`（`scope=full`）覆盖全部 job；发版构建改动再跑 `Release dry-run`。
- 额度用完或服务不可用时，使用 `just ci`，发布使用 `just release` / `just ship` 等本地命令。
- 推送 `main` 前运行本地 CI。不假定 push/tag 会触发 Actions；需要托管结果时明确手动触发并核对结果。

`CI` 的 `scope=auto` 按 `base_ref..HEAD` 选 job（默认 `HEAD^..HEAD`）；未知路径选择全部 job。
`Flutter Integration Tests` 默认 `suite=full`，`suite=smoke` 跑两个离线文件和 cloud-sync 文件。

## 本地覆盖范围

`just ci` 包含结构与增长门禁、脚本自测、Rust fmt/build/Clippy、真实 Postgres 测试、
Flutter 静态分析与全部单元/widget 测试、FFI 源码指纹验证、Web 构建、字体镜像校验、
浏览器专用 Flutter 套件，以及 rich-preview / clipboard / ui-experience / sidebar-rename 四个浏览器回归。
可分项运行 `just ci rust`、`just ci flutter`、`just ci e2e`；共有脚本门禁在每一项前执行。

浏览器回归使用 `e2e/package-lock.json` 钉死的 Playwright 及配套 Chromium。
本机不能下载时，设置 `CHROME_EXECUTABLE` 指向已有 Chrome/Chromium；指定路径无效会失败，不会静默换浏览器。
Linux 需安装 Chromium 的系统依赖（托管 runner 由 Playwright `--with-deps` 安装）。

以下仍为独立验收：真实 API/RustFS 的 `web_e2e.mjs` 与 `upload_length_e2e.mjs`、
容器栈启动、MCP inspector、Windows FFI 集成（`just test-integration`）。脚本结束时也会列出边界。

## 生产网络与凭据

部署、容灾和生产诊断均需要 SSH。生产节点有公网 SSH 入口；官方 runner 的只读生产诊断已实际通过，运行链接见本轮[验收记录](../reviews/redundancy-and-home-review-2026-11-10.md#6-第二轮实施与验收记录2026-11-11)。
如目标环境不允许官方 runner 连接，改走本地 `just deploy-prod` / 容灾流程；不要提交凭据或内网地址来绕过。
SSH 材料和 Docker 登录仍放 `$RUNNER_TEMP` 并清理：官方 runner 上同样能限定凭据生命周期。
发布两条完整路径见 [发布文档](release.md)。
