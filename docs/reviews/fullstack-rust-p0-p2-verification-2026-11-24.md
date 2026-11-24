# 全栈 Rust 架构评审（P0–P2）落地与验证总台账

> **日期**：2026-11-24  
> **分支**：`feat/fullstack-rust-architecture`（未合并 `main`，未触发正式发版）  
> **源评审**：[`origin/main:docs/reviews/fullstack-rust-branch-review-2026-11-24.md`](https://github.com/weironz/mica/blob/main/docs/reviews/fullstack-rust-branch-review-2026-11-24.md)  
> **实测证据**：
> - 终版完整 GitHub Actions CI（`scope=full`，含 A1b 评审修复及全部体验修复 `fbd91002`）：[Run 45427644316](https://github.com/weironz/mica/actions/runs/45427644316)（**7 / 7 任务全绿**：workspace **1,586**、`mica-editor` **74**、`mica-client` **34**、`clients/mica_rust` **131**、首包 Brotli-11 **`3,927,323 B` / 3.75 MiB**）
> - 前序完整 CI（`23bfecfa`）：[Run 45377675471](https://github.com/weironz/mica/actions/runs/45377675471)（**7 / 7 任务全绿**）
> - Release dry-run（`build_linux_deb=true`, `publish=false`）：[Run 45374949409](https://github.com/weironz/mica/actions/runs/45374949409)（**9 / 9 构建任务全绿**，未发布）

---

## 1. P0 / P1 / P2 逐项落地总表

| 编号 | 评审项 | 状态 | 关键提交 | 关键实现与验收证据 |
| --- | --- | --- | --- | --- |
| **P0-1** | 恢复 `#111` 账号隔离云端缓存键 | **已完成并验真** | `0804738f` | `crates/mica-client/src/cache_origin.rs` 规范化 `normalize_server_url` + `cloud_cache_origin(server_url, account_id)`；登录强制拉取 `/api/auth/me` 绑定 `account_id`；Playwright E2E `[2/6]` 在真实 Chromium IndexedDB `mica_sync_v1` 验证缓存键含账号隔离前缀 |
| **P0-2** | 删除 Web 同步 XHR + 接通 IndexedDB 离线持久化 | **已完成并验真** | `03981b66` | `crates/mica-client/src/net.rs` 全面改为异步 `fetch` + `AbortController` 超时 + `RequestCredentials::SameOrigin`；`crates/mica-client/src/web_idb.rs` 落地 `mica_sync_v1/docs` + Web Locks 串行写入；Playwright E2E `[1/6]` 验证 **0 XHR**，`[5/6]` 验证断网编辑 → IDB `outbox` → 刷新恢复 |
| **P0-3 (W1–W5)** | Web 首包压进 ≤ 4.0 MiB（Brotli-11）+ 启动体验与硬门禁 | **已完成并验真** | `8c100db7` | **W4**：Web `mica-md` 仅启用 `render-board`，公式/Mermaid 拆为按需懒加载 `mica_preview/{math,mermaid}.{js,wasm}`（经根 `Owner` + `nsh_core::prelude::spawn` 驱动）；**W2**：`wasm-opt -Oz`（`16.06 MB -> 13.93 MB`）；**W5**：`scripts/build-web-rust.sh` 硬门禁 ≤ `4,194,304 B`，CI 实测 **`3,926,345 B`（3.74 MiB）**（`math` gzip-9 `1,003,574 B`、`mermaid` gzip-9 `2,010,015 B`）；**W1**：内联 SVG 启动画面 + `modulepreload` / `preload as="fetch" crossorigin` |
| **P0-4** | Windows 自动更新 + 单实例互斥（`#179`）+ Inno `[InstallDelete]` | **已完成并物理机全链路演练通过** | `86fa49fc`、本次提交 | `clients/mica_rust/src/single_instance.rs`（`#179` 互斥锁，实测二次启动 113 ms 退出）；`clients/mica_rust/src/updater.rs`（GitHub Releases + SHA-256 校验 + `/MICAWAITPID`）；`clients/mica_rust/installer/mica.iss`（`[InstallDelete]` 含补清 `native_assets.json`）；已在真实 Windows 物理机完成 `v0.14.11` 覆盖升级 → 单实例 → 卸载 → 回滚 `v0.14.11` → `store.db` `PRAGMA quick_check = ok` 全链路演练 |
| **P1-1** | 明确并文档化平台交付范围 | **已完成** | `eb605e76` | 更新 `docs/architecture/fullstack-rust-architecture.md`（红线 3）、`README.md`、`README.zh-CN.md`、`docs/ops/release.md`：正式交付 Windows 桌面、Web、可选 Linux `.deb` 与跨平台 CLI；不交付 Android 与 macOS 桌面包，并在切换版发布说明标注 Android 止步 `0.14.x` |
| **P1-2** | 桌面中文输入法与多浏览器兼容性实测 | **自动化全绿；用户本机桌面+Web 真人实测确认通过** | `f497d4d9` | 已归档 [`fullstack-rust-ime-browser-verification-2026-11-24.md`](fullstack-rust-ime-browser-verification-2026-11-24.md)；2026-11-24 用户于物理 Windows 机器实测桌面端与 Web 端确认通过；未覆盖端（如 macOS Safari）与 `W-8` 安装包演练按实标注 |
| **P1-3** | 重建高价值编辑器回归与新 Web 端 E2E + 拉高 CI 回执下限 | **已完成并验真** | `3bdb9a9a`、`6daa44f7`、`23bfecfa` | **重质量不凑水测试**：`crates/mica-editor` 新增 46 条高价值行为回归 + `proptest`（26 → **72 条**），并顺带捕获修复 3 个真实编辑器 bug（空文档撤销掉 0 块、嵌套列表序号重置、空嵌套列表回车未退缩进）；`e2e/web_e2e.mjs` 6 大核心场景在 CI 真实容器栈全过；CI `cargo_receipt` 下限上调为 `--workspace ≥ 550`（实测 **1,581**）、`mica-editor ≥ 70`（实测 **72**）、`mica-client ≥ 30`（实测 **32**）、`clients/mica_rust ≥ 110`（实测 **129**） |
| **P1-4** | 修复 Linux GTK4/GStreamer CI 依赖并跑通完整 CI 与 Release dry-run | **已完成并验真** | `6daa44f7`、`d71011a1`、`23bfecfa` | 修复 `.github/workflows/{ci,release,release-dryrun}.yml`、`crates/cli/src/backup.rs` Linux `Stdio` 告警、升级 `url 2.5.9` / `cookie 0.18.3` 并登记 `nsh 0.6.1` 传递依赖 RustSec 豁免、恢复官方 `@modelcontextprotocol/inspector@1`（**39 个 MCP 工具通过**）、修复 `e2e/docker-compose.from-source.yml`；完整 CI（[45377675471](https://github.com/weironz/mica/actions/runs/45377675471)）7/7 与 Release dry-run（[45374949409](https://github.com/weironz/mica/actions/runs/45374949409)）9/9 双双全绿 |
| **P1-5** | 消除内联脚本 `'unsafe-inline'`，落地精确 `sha256` CSP 校验 | **已完成并验真** | `8c100db7` | `deploy/nginx.conf` 与 `deploy/nginx.dev.conf` 使用 `'sha256-RBDgFD9prAEUa1zzW08ImyUTCJYoF2SOC6nga7ZjqJ0='`（零 `'unsafe-inline'`），由 `scripts/test-web-csp-hashes.sh`、`scripts/web-entry-cache-check.sh` 与 Playwright E2E（0 CSP report）三层锁死 |
| **P2-1** | 清理 L2/L3 分层边界（在 P0 验收后再按需下沉） | **已完成并验真** | `f8a91686` | 将零 UI 依赖的 `storage.rs`、`net.rs`、`ws_transport.rs`、`web_runtime.rs`、`web_idb.rs`（约 6,000 行、0 `nsh-*` 依赖）整体下沉至 `crates/mica-client/src/`，`clients/mica_rust` 保留薄重导出；`mica-client` 32/32 单测 + native/wasm32 双目标 0 告警通过 |
| **P2-2** | 精确锁定 `nsh` 依赖版本与升级治理清单 | **已完成** | `eb605e76`、`d71011a1` | `clients/mica_rust/Cargo.toml` 全部 `nsh-*` 锁定 `=0.6.1`；新增 [`docs/dev/nsh-dependency-governance.md`](../dev/nsh-dependency-governance.md) 登记升级检查表与 8 项上游 `nsh 0.6.1` 已知问题/绕过方案 |

---

## 2. CI 与彩排关键实测数据（2026-11-24）

### 2.1 完整 CI（[Run 45377675471](https://github.com/weironz/mica/actions/runs/45377675471)，commit `23bfecfa`）

| Job | 耗时 | 实测结果 |
| --- | --- | --- |
| `Scope` | 6s | 全量展开 `rust`, `client`, `container`, `mcp` |
| `Structure & Hygiene` | 7s | `check-ignored-sources.sh`、`check-migration-tracking.sh`、`test-migration-tracking.sh`、`check-file-size.sh` 全绿 |
| `RustSec (both lockfiles)` | 3m21s | `Cargo.lock`（528 crates）与 `clients/mica_rust/Cargo.lock`（766 crates）0 漏洞 0 告警 |
| `Rust Workspace` | 3m21s | `rustfmt`、`build`、`clippy -D warnings`、Postgres 迁移、`--workspace` **1,581 passed**（下限 550）、`mica-editor` **72 passed**（下限 70）、`mica-client` **32 passed**（下限 30）、`mica-core --features store` **232 passed**、4 个 `mica-md` examples 全绿 |
| `Pure-Rust Client` | 6m31s | Ubuntu GTK4+GStreamer `clippy -D warnings`、**129 passed**（下限 110）、原生二进制、`test-web-csp-hashes.sh`、`build-web-rust.sh`（首包 br-11 **`3,926,345 B`** ≤ `4,194,304 B`）全绿 |
| `MCP` | 58s | 官方 `@modelcontextprotocol/inspector@1` 验真：**39 tools accepted** |
| `Container` | 13m19s | `Dockerfile.{web,cli,api}` 全建、`web-entry-cache-check.sh` PASS、Playwright `e2e/web_e2e.mjs` **6/6 PASS（0 console errors, 0 CSP reports, 0 XHR）** |

### 2.2 Release dry-run（[Run 45374949409](https://github.com/weironz/mica/actions/runs/45374949409)，commit `d71011a1`，`publish=false`）

| Job | 耗时 | 产物与验证 |
| --- | --- | --- |
| `version` + `build / draft` | 13s | 版本一致性 `0.14.11`、CLI 矩阵解析 |
| `build / windows` | 22m42s | `test-installer-wait.ps1` + `mica-rust.exe` release + Inno Setup `Mica-Setup-0.14.11.exe` + SHA-256 |
| `build / linux-deb` | 8m31s | `Mica-0.14.11-linux-amd64.deb` + SHA-256 |
| `build / cli (windows-latest)` | 4m16s | `mica-cli-0.14.11-windows-x64.exe` + SHA-256 |
| `build / cli (ubuntu-latest musl)` | 2m8s | `mica-cli-0.14.11-linux-x64` 静态二进制 + SHA-256 |
| `build / images (api, web, cli)` | 1m47s–10m4s | 3 个生产 Docker 镜像全部构建通过（`publish=false`，未推送） |

---

## 3. 演练额外收口与合并前状态

所有 P0–P2 项、完整 CI（7/7）、Release dry-run（9/9）、用户本机桌面/Web 实测以及 **P0-4 真实覆盖安装与回滚演练**均已全部闭环通过；且演练期间额外发现并修复了 2 个真实生产体验问题：

1. **Inno Setup `[InstallDelete]` 遗漏 `native_assets.json`**：已在 `clients/mica_rust/installer/mica.iss` 补齐。
2. **桌面端未缓存云端页首次打开 1~2 秒白屏**：通过全局复用 `ureq::Agent` 连接池（Keep-Alive + TLS 复用）与 WS 共享 `rustls::ClientConfig`、移除大工作区切页全量同步扫库，并在未命中本地缓存时显示骨架屏加载态彻底消除，配 2 条单测回归锁定。

分支 `feat/fullstack-rust-architecture` 已具备合并 `main` 条件（等待用户明确指示是否合并及发版）。
