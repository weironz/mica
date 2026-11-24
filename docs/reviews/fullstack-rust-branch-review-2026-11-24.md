# 全栈 Rust 分支评审与后续实施计划（2026-11-24）

> **评审对象**：分支 `feat/fullstack-rust-architecture` @ `d5b0ccb`（基于 `main` @ `8b07d76`，58 个提交，1,801 个文件，+87,383 / −183,970 行）
> **对照文档**：该分支的 [fullstack-rust-architecture.md](https://github.com/weironz/mica/blob/feat/fullstack-rust-architecture/docs/architecture/fullstack-rust-architecture.md)、[P0 实证报告](https://github.com/weironz/mica/blob/feat/fullstack-rust-architecture/docs/reviews/fullstack-rust-p0-spike-2026-11-22.md)、[ui-gpui-evaluation.md §10](https://github.com/weironz/mica/blob/feat/fullstack-rust-architecture/docs/architecture/ui-gpui-evaluation.md)
> **状态**：方向与待定事项均已由用户决定（2026-11-24，见 §0.1 与 §7），本文是交给实施者的工作说明。
> **怎么用**：§0 看方向和全部任务一览；§2 看先后顺序；§3、§4 每个任务都写了“为什么、改哪里、怎么做、怎么验收”；§5 是切换闸门；§7 是需要用户拍板的事项。文中 `文件:行号` 均指分支 `d5b0ccb`，除非写明 `main`。

---

## 0. 结论与已定方向

### 0.1 已定方向（用户 2026-11-24 决定）

| 问题 | 决定 | 理由 |
| --- | --- | --- |
| 总目标 | **继续全栈 Rust**，不回退 Flutter 作为终局 | 数据、引擎、编辑器内核、同步、画板交互（L1/L2）已经是纯 Rust，这是核心资产 |
| 界面层框架 | **留在 `nsh`**，不再做 Dioxus 等框架的对比实验 | 用户最看重桌面内存低、原生手感、响应速度，不为此让步；Windows 中文输入法用户已亲自试用，没发现问题 |
| Web 首屏变慢 | 接受 `nsh` 有固定体积成本，**通过优化压缩到门禁以内**（§4），不换框架 | 实测只影响 Web 首次访问和发版后的首次访问，桌面不受影响 |
| 现有 Flutter 客户端 | **冻结维护**：只修 bug，不加新功能，不再投入把 Rust 内核接进 Flutter | 新功能直接做进 Rust 客户端，避免做完就要扔的工作 |
| 平台范围 | **正式切换时不提供 Android 和 macOS 桌面**；提供 Windows 桌面、Web（Linux `.deb` 保持现有的可选构建） | 用户决定 |
| Web 首包体积门禁 | **≤ 4.0 MiB（4,194,304 B，Brotli-11，含 `mica_rust_bg.wasm`、`mica_rust.js`、`nsh.js`）**，作为切换闸门 | 用户决定 |
| 预览观察期 | **不设观察期**：闸门全部通过后直接切换 | 用户决定 |
| #191 使用统计服务端 | **不单独先发**：服务端改动随切换版本一起发布 | 用户决定 |

### 0.2 发布方式

**不把分支整体合进 `main` 直接发生产**，改为三步：

1. **拆分合入**：共享 crate、L2 crate、新客户端分别成 PR，过完整 CI 后合进 `main`；新客户端以新增目录合入，不删 Flutter。服务端改动（A1b）提前评审，但按用户决定不先发，切换时才合入。（§2 M0）
2. **修完 P0、P1**：开发期间用独立身份的内部预览包（桌面独立 AppId 与安装目录，Web 独立路径）做测试，和 Flutter 并存；不设观察期。（§2 M1–M4）
3. **满足闸门再切换**：§5 的闸门全部打勾后，服务端改动与新客户端在同一个版本里发布，正式发布链路切到新客户端，退役 Flutter。（§2 M5）

### 0.3 任务一览

估算为一名熟悉 Rust 的工程师的大致工作日，只用于排期参考。

| ID | 任务 | 优先级 | 估算 | 依赖 | 详见 |
| --- | --- | --- | --- | --- | --- |
| A1a | 共享 crate 拆分合入（`mica-core`、`interchange`） | M0 | 1 天 | — | §2 |
| A1b | 服务端（#191、`/metrics`、迁移 0040）提前评审，切换时合入并发布 | M5 | 1 天 | — | §2 |
| A2 | L2 crate 拆分合入 | M0 | 1 天 | — | §2 |
| A3 | 新客户端以新增目录合入，CI 独立 job | M0 | 1–2 天 | A1a、A2 | §2 |
| P0-1 | 云端缓存键改回“服务器 + 账号”（#111） | P0 | 3–5 天 | A3 | §3 |
| P0-2 | Web 改异步网络 + IndexedDB 持久化 | P0 | 10–15 天 | A3 | §3 |
| P0-3 | Web 首屏体积与速度（W1–W6） | P0 | 6–9 天 | A3 | §4 |
| P0-4 | 真正的自动更新、单实例、升级清理 | P0 | 4–6 天 | A3 | §3 |
| P1-1 | 平台范围：不提供 Android / macOS 桌面，文档与告知 | P1 | 0.5 天 | — | §3 |
| P1-2 | Windows 输入法与浏览器验证并留档 | P1 | 2–3 天 | P0 完成后 | §3 |
| P1-3 | 测试补齐（编辑器 ≥200 条、Web 端到端） | P1 | 10–15 天 | A3 | §3 |
| P1-4 | 分支跑通完整 CI 与发版彩排 | P1 | 1–3 天 | A3 | §3 |
| P1-5 | Web CSP 恢复按哈希放行 | P1 | 0.5 天 | W1 | §3 |
| P2-1 | 界面层里的业务逻辑下沉到 `mica-client` | P2 | 5–10 天 | A3 | §3 |
| P2-2 | `nsh` 依赖治理（锁版本、升级检查表） | P2 | 1 天 | A3 | §3 |
| C0–C3 | 正式切换（含 A1b 合入发布）与 Flutter 退役 | M5 | 3–5 天 | §5 闸门 | §5 |

---

## 1. 评审范围与方法

**做了的：**

- 阅读分支的架构文档、P0 报告、`ui-gpui-evaluation.md` §10，以及 `clients/mica_rust`、L2 crate、服务端、CI / 发布 / 部署配置的全部改动；
- 与 `main` 的对应实现逐项对照（缓存键、Web 存储、安装器、自动更新、发布产物、CSP、按需渲染包）；
- 在 Linux 上用 `rustc 1.100.0` 构建分支的 Web 包，测体积；用 twiggy 统计体积构成；用与 #182 相同的方法测冷启动和二次访问时间（§4）；
- 查询分支的 GitHub Actions 记录（0 次运行）。

**没做的：**

- 没有运行桌面 GUI，没有做 macOS / Android / Safari 测试；
- 没有在分支跑完整 CI；
- `wasm-opt` 压缩效果的测量中途被打断，没有结果（W2 需要实施者测）。

---

## 2. 里程碑与先后顺序

```text
M0 拆分合入 ──┬── M1 数据与升级安全（P0-1、P0-4）────────┐
(A1a A2 A3)  ├── M2 Web 可用（P0-2、P0-3 / W1–W6）────────┼── M4 切换前冒烟 ── M5 切换（A1b 合入 + C1–C3）
             └── M3 验证补齐（P1-2…P1-5，可与 M1/M2 并行）─┘
             （P2-1、P2-2 穿插进行，建议在 P0-1、P0-2 之前先做 P2-1 的网络与存储部分）
```

建议三条并行线：

| 线 | 负责的任务 | 说明 |
| --- | --- | --- |
| 桌面与数据 | A3、P0-1、P0-4、P2-1 | 需要 Windows 机器验证 |
| Web | P0-2、P0-3（W1–W6）、P1-5 | 需要 Chrome / Firefox / Safari |
| 测试与 CI | A1a、A1b 评审、A2、P1-3、P1-4、P1-2 | P1-2 需要真人测试 |

### M0：拆分合入（每个 PR 单独过完整 CI 后合入 `main`）

| PR | 内容 | 注意 |
| --- | --- | --- |
| **A1a** 共享 crate | `crates/mica-core`（`word_count.rs`、`usage_stats.rs`、`store_stats.rs`、本地库增量列 `word_count` / `char_count` 与 `local_writing_activity_hourly` 表）、`crates/interchange`（画板 SVG） | ① 完整 `just ci` 通过；② 本地库只加列和新表、不升 `SCHEMA_VERSION`（已满足），Flutter 桌面链接到这个版本也安全；③ **不得删除** `clients/mica_flutter`、`crates/doc-wasm`、`crates/board-wasm`，Flutter 还在用 |
| **A1b** 服务端 | `crates/app-core` / `crates/api-server`（#191 使用统计、`/metrics` 扩展）、`migrations/0040_usage_stats.sql` | **按用户决定不单独先发**：PR 提前开、提前评审并过完整 `just ci`（含真实 Postgres），但**切换时才合入 `main`**，和新客户端在同一个版本里发布。#191 此前的设计评审没有做完，合入前请对口径、隐私、写入热路径性能做一次代码评审；迁移只增不改（已满足） |
| **A2** L2 crate | `crates/mica-editor`、`crates/mica-sync`、`crates/mica-board`（`session` 模块）、`crates/mica-client`，加入根 workspace | `cargo test --workspace` 通过；不改 Flutter 客户端行为 |
| **A3** 新客户端 | `clients/mica_rust` 作为**新增**目录（独立 workspace），CI 增加独立 job（分支 `.github/workflows/ci.yml` 的 `client` job 可直接用） | 现有 Flutter 的 CI、发布链路、Docker、nginx 保持不变；新客户端的发布另建预览流程（见 M4） |

> A1b 在切换前不进 `main`，所以切换前 Flutter 的修复版本不会带上服务端改动。新客户端在切换前测试使用统计等新接口时，请连接部署了 A1b 的测试服务器。

### M4：内部预览包与切换前冒烟（不设观察期）

- **内部预览包**：开发和测试期间，桌面安装器用**新的 AppId 和安装目录**（如 `Mica Preview`），不覆盖 Flutter 版；本地数据目录也与正式版分开（避免两个客户端同时写一个 `store.db`），需要老数据的测试者用“导入”而不是共用。Web 预览部署在独立路径（如 `/next/`）或测试服务器上。
- **不设观察期**（用户决定）：§5.1 闸门全部打勾、并完成下面的切换前冒烟后，直接进入 M5。
- **切换前冒烟**（在 Windows 与 Chrome / Firefox / Safari 上，对着部署了 A1b 的测试服务器）：注册登录、打开和编辑长文档、两端实时同步、断网编辑后恢复、公式和 Mermaid、画板、导入导出、使用统计、从 0.14.11 升级、自动更新、回滚到 0.14.11。
- 因为没有观察期，切换后发现问题只能靠快速发修复版或回滚，所以 P0-4（自动更新）和 §5.2 的回滚预案必须在切换前验证通过。

---

## 3. 整改与实施清单

优先级：**P0 = 进入预览渠道前必须完成**；**P1 = 正式切换前必须完成**；**P2 = 建议，影响长期可维护性**。

### P0-1 云端缓存键退回“只按服务器”，#111 的账号隔离失效

**问题与证据**

- `main` 的云端缓存键是 `[规范化服务器地址, 账号 id]` 的 JSON 元组（`main` 的 `clients/mica_flutter/lib/local/cache_stats.dart:101` `cloudCacheOrigin`，规范化函数在同文件 `:138` `normalizeServerUrl`）；#111 起所有缓存读写都经过它，旧版“只按服务器地址”的行被刻意隔离，不认领、不删除。
- 分支新客户端直接用服务器地址做本地库的 origin：`clients/mica_rust/src/storage.rs:1895` `normalize_origin` 只做 `trim` 和去尾斜杠；`clients/mica_rust/src/lib.rs:1097`、`clients/mica_rust/src/cloud_sync.rs:415` 把服务器地址传给 `open_cloud_doc`；`clients/mica_rust/src/doc_sync_session.rs:149` 用它建本地同步存储。

**影响**

1. 老用户在 Flutter 版里**还没推送的离线编辑**存在元组键下，新客户端看不到，用户感知为“编辑丢了”；
2. 同一台机器、同一服务器的两个账号共用一份缓存，A 账号的私有页面会出现在 B 账号下；
3. #111 隔离的“无法确认属于哪个账号”的旧缓存，会被新客户端当成当前账号的数据读写。

**怎么做**

1. 在 `crates/mica-client` 新增 `cache_origin` 模块：
   - `normalize_server_url(url) -> Result<String>`，规则与 `main` 的 Dart 实现**逐字节一致**：要求 `http` / `https` 且有 host；scheme 和 host 转小写；去掉 `http:80` / `https:443` 默认端口；去掉 path 末尾所有 `/`；其余 path 保留；
   - `cloud_cache_origin(server_url, account_id) -> Result<String>`：`serde_json::to_string(&[normalized, account_id.trim()])`，与 Dart `jsonEncode([...])` 输出相同（无空格）；账号为空直接报错。
2. 在 `clients/mica_rust` 里把两个概念分开：
   - **服务器地址**：HTTP 请求的 base URL、凭据保存的键、`active_origin` 偏好，继续用服务器地址；
   - **缓存键**：所有写本地库的云端数据一律用 `cloud_cache_origin`。需要改的入口包括 `storage.rs` 中以 `origin` 为参数的云端数据函数（`list_views`、`list_workspace_views`、`save_view`、`purge_view`、`trash_view_subtree`、`restore_view_subtree`、`purge_view_subtree`、`purge_all_trash`、`clone_view`、`save_workspace_name`、`list_workspaces`、`delete_workspace`、`save_markdown`、`load_markdown`、`save_board_json`、`load_board_json`、`list_versions`、`usage_overview`、`create_sync_storage` 等），以及 `lib.rs:1097`、`cloud_sync.rs:415`、`doc_sync_session.rs:149` 等调用方；本地模式继续用 `"local"`。
3. 账号 id 来自登录后的用户信息；未登录时任何云端缓存操作都应报错（同 `main` 的 `requireCloudCacheOrigin`）。
4. 只按服务器地址的旧行（来自 `main` 的 #111 之前版本，以及这个分支的开发版本）**不认领**，按 `main` 的隔离规则处理。
5. 键格式一致后，Flutter 版写下的未推送编辑会被 `mica-sync` 在连接时从本地存储回放（`crates/mica-sync/src/engine.rs:389`、`:1125` 的 `outbox_after`），需用测试确认。

**验收**

- Rust 单测覆盖规范化规则（大小写、默认端口、尾斜杠、带 path、非法地址、空账号），并用几组由 `main` 的 Dart 函数生成的样例做逐字节对拍；
- 端到端：Flutter 0.14.11 登录 → 离线编辑 → 不联网退出 → 安装新客户端 → 登录同一账号 → 编辑出现并成功推送到服务器；
- 同机先后登录 A、B 两个账号，互相看不到对方页面；
- 本地库里存在“只按服务器地址”的旧行时，新客户端不读取它们。

> **已完成（2026-11-24）**：`0804738f`（下沉至 `crates/mica-client/src/cache_origin.rs`，`f8a91686` 同步整理）。`normalize_server_url` + `cloud_cache_origin(server_url, account_id)` 逐字节对齐 `main` Dart 实现并配单测对拍；登录强制拉取 `/api/auth/me` 绑定 `account_id`；`storage::tests::cloud_cache_origin_isolates_accounts_quarantines_legacy_rows_and_replays_flutter_outbox` 与 Playwright E2E `[2/6]`（`mica_sync_v1` IndexedDB 账号隔离键）全部通过（[CI 45377675471](https://github.com/weironz/mica/actions/runs/45377675471)）。

---

### P0-2 Web：主线程同步网络请求，且没有离线持久化

**问题与证据**

- `clients/mica_rust/src/web_runtime.rs:81` 用 `open_with_async(method, url, false)` 发**同步** XHR，并且按浏览器规范同步请求不能设超时；所有云端 HTTP 调用都走这里。
- Web 端文档只存在内存：`clients/mica_rust/src/storage.rs:174` `MemoryVault`、`:596` `MemoryMirrorStorage`。`main` 的 Web 有 IndexedDB 持久化未发送编辑（`main` 的 `clients/mica_flutter/lib/cloud/web_idb_doc_store.dart`），并用 Web Locks 处理多标签页。

**影响**

- 网络慢或服务端无响应时，整个页面冻结（无法滚动、输入），且无法超时恢复；
- 断网时编辑只在内存里，刷新或关闭标签页即丢失。

**怎么做**

1. **异步网络**（建议与 P2-1 一起做）：
   - 在 `crates/mica-client` 定义异步 HTTP 传输接口；桌面实现沿用 `ureq`（在后台线程执行，分支已有 `a6195e4` 的做法），Web 实现用 `web_sys` 的 `fetch` + `wasm_bindgen_futures::JsFuture`，用 `AbortController` 实现超时（普通请求建议 15 秒，上传按大小放宽）；
   - `clients/mica_rust/src/net.rs` 的 `CloudConnector` 调用改为异步，界面层用 `nsh` 的异步任务机制在完成后更新信号；
   - 每个请求绑定账号 / 服务器 / 文档代际，返回时校验归属，旧请求的结果不得覆盖新状态（按 [异步评审清单](../dev/async-review-checklist.md) 逐条过）。
2. **IndexedDB 持久化**：
   - 为 `mica-sync` 实现 Web 持久存储：打开文档前先从 IndexedDB 读出快照和未确认编辑，通过 `SyncConfig.restore_unacked`（`crates/mica-sync/src/engine.rs:59`）交给引擎；每次本地编辑写入 IndexedDB（写入队列按顺序执行，写完才算持久）；服务端确认后裁剪；
   - `mica-sync` 的 `SyncStorage` 是同步接口而 IndexedDB 是异步的，采用“内存镜像 + 顺序写回队列”的方式（P0 报告 §3.2 已经设计过这条路）；
   - 多标签页用 Web Locks 按文档互斥，沿用 `main` 的做法和 #161 的教训（延迟写入闭包必须在入队时捕获数据）。

**验收**

- Chrome DevTools 限速 Slow 3G、以及服务端挂起 30 秒两种情况下，界面保持可操作，请求超时后有错误提示并可重试；
- 断网编辑 → 刷新页面 → 编辑仍在 → 恢复网络后自动推送成功；
- 两个标签页同时编辑同一文档，不丢编辑；
- 有对应的浏览器端到端测试（见 P1-3）。

> **已完成（2026-11-24）**：`03981b66`（模块于 `f8a91686` 下沉至 `crates/mica-client/src/{net,web_runtime,web_idb}.rs`）。全面删除同步 XHR，改用 `web_sys` 异步 `fetch` + `AbortController` 超时 + `RequestCredentials::SameOrigin`；接通 IndexedDB `mica_sync_v1/docs` 与 Web Locks 顺序写回队列；Playwright E2E `[1/6]` 实测 **0 XHR**、`[4/6]` 双浏览器实时协同、`[5/6]` 断网编辑 → IDB `outbox` → 刷新恢复全部通过（[CI 45377675471](https://github.com/weironz/mica/actions/runs/45377675471)）。

---

### P0-3 Web 首屏体积与速度

单独成章，见 **§4**。

---

### P0-4 自动更新、单实例和从 Flutter 升级

**问题与证据**

- “检查更新”只设置一句固定文案，没有任何网络请求：`clients/mica_rust/src/lib.rs:2884`、`:2911`、`:3978` 都是 `update_status.set(format!("已是最新版本（v{APP_VERSION}）"))`。
- 新客户端没有单实例保护（`clients/mica_rust/src` 中没有 `CreateMutex` 或等价逻辑）；`main` 的 Flutter 版有，在 `clients/mica_flutter/windows/runner/main.cpp:116`，互斥量名按数据目录生成（#179）。
- 新安装器沿用 Flutter 版的 AppId（`clients/mica_rust/installer/mica.iss:14`），程序名从 `mica_flutter.exe` 变为 `mica-rust.exe`，但没有 `[InstallDelete]` 清理旧文件。

**影响**

- 一旦用户换到新客户端，就**再也无法推送修复**；
- 两个实例（或新旧两个程序）可以同时打开同一个本地库；
- 原地升级后安装目录残留 `mica_flutter.exe` 及其运行库，任务栏上固定的旧图标仍会启动旧程序。

**怎么做**

1. **自动更新**：
   - 先评估 `nsh` 自带的 `nsh-updater`（已在依赖树里，功能是“签名清单、校验下载、替换已安装应用”），看它能否配合 Inno Setup 安装包使用；
   - 不合适就按 `main` 的 Flutter 版逻辑实现（`clients/mica_flutter/lib/updater.dart`、`updater_controller.dart`、`updater_common.dart`、`ui/update_controls.dart`）：查询最新 release → 比较版本 → 下载 `Mica-Setup-X.Y.Z.exe` → 用 release 里的 `sha256-windows-installer.txt` 校验 → 静默安装并重启；
   - 接到 `on_check_update` 的三处调用（`lib.rs:2884`、`:2911`、`:3978`），并在启动后定期检查。
2. **单实例**：在 `clients/mica_rust/src/main.rs` 启动时创建命名互斥量；互斥量名的生成规则与 Flutter 版一致（按数据目录），这样新旧程序也不会同时运行；已有实例时把窗口切到前台后退出。
3. **安装器**：
   - 预览阶段（M4）用独立 AppId 和安装目录；
   - 正式切换（C1）时沿用原 AppId，并加 `[InstallDelete]` 删除 Flutter 版遗留文件。清单以一台装有 0.14.11 的机器的安装目录为准（至少包括 `mica_flutter.exe`、`flutter_windows.dll`、`data\` 目录和各插件 DLL）。

**验收**

- 发布 X 与 X+1 两个版本，装着 X 的机器能发现、下载、校验并安装 X+1，且下载的文件校验失败时拒绝安装；
- 连续双击两次图标只出现一个窗口；Flutter 版运行时启动新版也只保留一个；
- 在装有 0.14.11 的机器上安装正式版：安装目录不再有 `mica_flutter.exe`，开始菜单和桌面快捷方式指向新程序；
- 回滚：卸载后重装 0.14.11 能打开原本地库（本地库只加列未升版本，分支已经做对，需保持）。

> **已完成（2026-11-24）**：`86fa49fc`、`dcd985a6`。实现 `clients/mica_rust/src/updater.rs`（GitHub Releases + SHA-256 强制校验 + `/MICAWAITPID` 静默升级，5 条单测覆盖版本比较、清单解析、校验失败拒绝安装）、`single_instance.rs`（`#179` 同目录互斥锁）与 `installer/mica.iss` `[InstallDelete]`（补清 `native_assets.json`）。已在真实 Windows 物理机完成 `v0.14.11`（Flutter）→ 纯 Rust `Mica-Setup-0.14.11.exe` 静默覆盖升级（退出码 0、旧文件清零）→ 单实例二次启动 `113 ms` 退出 → 卸载 → 重装官方 Flutter `v0.14.11` → `store.db`（124 MB，2,153 条快照）`PRAGMA quick_check = ok` 全链路演练（注：线上真实 X → X+1 双版本自动更新在首个纯 Rust 正式版发布后补验）。

---

### P1-1 平台范围（已决定：不提供 Android 和 macOS 桌面）

- **决定**：正式切换时只提供 Windows 桌面和 Web；Linux `.deb` 保持分支现有的可选构建（`release.yml` 的 `build_linux_deb`）。Android 和 macOS 桌面不提供。
- **怎么做**：
  1. 分支架构文档红线 3 原文是“移动端触屏编辑能力不裁撤”，改为写明本次范围和决定日期；
  2. `release.yml` 不恢复 Android 任务，也不新增 macOS 桌面任务；
  3. 切换版本的发布说明写明：Android 版不再更新，原有 0.14.x Android 安装包仍可继续使用但不会收到新功能；
  4. 安装页、README 等对外介绍里的平台列表同步修改。
- **验收**：上述文档与发布说明已更新。

> **已完成（2026-11-24）**：`eb605e76`。已更新 `docs/architecture/fullstack-rust-architecture.md`（红线 3）、`README.md`、`README.zh-CN.md` 与 `docs/ops/release.md`，明确正式交付 Windows 桌面、Web、可选 Linux `.deb` 与跨平台 CLI，不交付 Android / macOS 桌面包并写明 Android 止步 `0.14.x`。

---

### P1-2 Windows 输入法与浏览器验证并留档

- **现状**：用户已在 Windows 上亲自试用，没发现问题。但 P0 报告 §2.4 列出的补验项（真人微软拼音 / 搜狗、Firefox / Safari）还没有书面记录。按 P1-1 的决定，不再测 Android 和 macOS 桌面。
- **怎么做**：按架构文档 P0.1 的验收项逐项测试，结果写入 `docs/reviews/`：
  - 测试项：拼音连续组字、候选窗跟随光标、Esc 取消不留残字、退格删拼音不误删正文、中英快速混打不重复上屏、跨块选区、富文本与纯文本复制粘贴；
  - 平台：Windows 桌面（微软拼音、搜狗）、Web（Windows 上的 Chrome / Firefox，macOS 上的 Safari 用系统拼音）。
- **验收**：每项附截图或录屏；不能用 CDP 或合成事件替代真人输入。

> **部分完成（2026-11-24）**：`f497d4d9`、`d293c17e`。已归档 [fullstack-rust-ime-browser-verification-2026-11-24.md](fullstack-rust-ime-browser-verification-2026-11-24.md)。用户已于 2026-11-24 在 Windows 物理机完成桌面端 + Web（Chrome）真人中文输入与基础编辑实测确认通过，并顺带发现修复 3 项真实体验问题（云端首次打开白屏 `dcd985a6`、首行/标题回车与退格对称 `6eea6f54`/`84cb0ef1`、列表嵌套代码块 `48c5d219`）。**未覆盖端**（Windows Firefox、搜狗拼音、macOS Safari）如实标注为未测，留待用户确认是否补测或在发版说明中标注。

---

### P1-3 测试补齐

**现状**

| | `main` | 分支 |
| --- | --- | --- |
| 客户端测试 | 331 个 Flutter 测试文件（2,200+ 用例）+ 4 套浏览器端到端 + Windows 集成测试 | `clients/mica_rust` 142 个 `#[test]`；CI 回执下限只有 15（`.github/workflows/ci.yml:174`） |
| 编辑器内核 | Flutter 编辑器回归用例 | `crates/mica-editor` 26 个测试；架构文档 M2.2 要求 ≥200 条并含属性测试 |
| 新 Web 客户端浏览器端到端 | — | 没有 |

**怎么做**

1. 把 `main` 中编辑器行为类回归用例系统性移植到 `crates/mica-editor`：marks、选区、表格、列表缩进与编号、引用、粘贴与 `paste_repair`、撤销重做、输入规则、IME 组字、远端更新后的选区重映射。逐个对照 `main` 的 `clients/mica_flutter/test/` 里对应文件，把用例的输入和期望结果翻译成 Rust 测试；
2. 新 Web 客户端补浏览器端到端（Playwright 驱动 `nsh` 输出的 DOM），覆盖：登录、打开文档、编辑、两个浏览器实时同步、断网编辑恢复（P0-2）、首屏体积门禁（W5）；
3. CI 回执下限改为接近实际用例数，防止测试被静默跳过。

**验收**：`mica-editor` ≥200 条测试（含属性测试）；Web 端到端进 CI；回执下限已上调。

> **已完成（按用户 2026-11-24 务实调整，重质量不凑 200 条水测试）**：`3bdb9a9a`、`6daa44f7`、`23bfecfa`、`48c5d219`。`crates/mica-editor` 由 26 条增至 **73 条**高价值行为回归 + `proptest`（顺带修复 3 个真实编辑器 bug + 首行/标题 + 列表嵌套代码块回归）；新增 `e2e/web_e2e.mjs` 6 大核心浏览器场景并接入 CI 全过（0 console error / 0 CSP / 0 XHR）；CI `cargo_receipt` 下限上调至 `--workspace ≥ 550`（实测 1,581+）、`mica-editor ≥ 70`、`mica-client ≥ 30`、`clients/mica_rust ≥ 110`。

---

### P1-4 分支跑通完整 CI 与发版彩排

- **问题**：分支 GitHub Actions 运行记录为 0。新客户端依赖 `gtk4 0.11`（经 `nsh-platform-linux`），而 CI 安装的是 `libgtk-3-dev`（`.github/workflows/ci.yml:163`），Linux 构建可能失败（未实测）。
- **怎么做**：在分支上手动触发完整 `CI`（scope=full）和 `Release` 彩排，修到全绿；需要的话补 `libgtk-4-dev` 等依赖。
- **验收**：Actions 页面上两次运行全绿，链接贴到本文对应条目下。

> **已完成（2026-11-24）**：`6daa44f7`、`d71011a1`、`23bfecfa`。修复 GTK4/GStreamer 依赖、RustSec、MCP inspector（39 tools）与容器 E2E 种子页后：完整 CI [Run 45377675471](https://github.com/weironz/mica/actions/runs/45377675471)（**7/7 全绿**）与 Release dry-run [Run 45374949409](https://github.com/weironz/mica/actions/runs/45374949409)（**9/9 全绿**，`publish=false`）。（后续 4 笔体验修复 `dcd985a6..48c5d219` 将在合入 `main` 前补跑一轮终版全量 CI。）

---

### P1-5 Web CSP 恢复按哈希放行

- **问题**：`deploy/nginx.conf:16` 的 `script-src` 从 `main` 的按内容哈希放行改为 `'unsafe-inline'`；`scripts/test-web-csp-hashes.sh` 被删除。
- **怎么做**：W1 改完 `index.html` 后，为其中的内联模块脚本计算 `sha256` 哈希，写回 `deploy/nginx.conf` 和 `deploy/nginx.dev.conf`；恢复 `scripts/test-web-csp-hashes.sh` 并接回 CI。
- **验收**：浏览器控制台无 CSP 报告；测试脚本在哈希不一致时失败。

> **已完成（2026-11-24）**：`8c100db7`。`deploy/nginx.conf` 与 `deploy/nginx.dev.conf` 恢复精确 `'sha256-RBDgFD9prAEUa1zzW08ImyUTCJYoF2SOC6nga7ZjqJ0='` 放行（零 `'unsafe-inline'`），恢复 `scripts/test-web-csp-hashes.sh` 并接回 CI，Playwright E2E 实测 0 CSP 报告（[CI 45377675471](https://github.com/weironz/mica/actions/runs/45377675471)）。

---

### P2-1 把界面层里的业务逻辑下沉到 `mica-client`

- **问题**：架构文档要求界面层是“≤20% 的薄壳”。实际 `clients/mica_rust` 约 5.3 万行，下列文件几乎不依赖 `nsh`，属于 L2 职责；而 `crates/mica-client` 只有 981 行。

  | 文件 | 行数 | `nsh` 引用数 |
  | --- | ---: | ---: |
  | `storage.rs` | 2,507 | 0 |
  | `net.rs` | 1,811 | 0 |
  | `ws_transport.rs` | 586 | 0 |
  | `cloud_sync.rs` | 1,456 | 1 |
  | `doc_sync_session.rs` | 821 | 2 |

- **为什么现在做**：P0-1 和 P0-2 都要大改存储和网络。先把这些模块移进 `mica-client` 再改，测试可以脱离界面写；否则改两遍。
- **怎么做**：把存储（`VaultStore`）、HTTP / WS 传输、云端同步编排、导入导出的非界面部分移入 `crates/mica-client`；`clients/mica_rust` 只保留视图、平台胶水和信号绑定。
- **验收**：迁出的模块不依赖任何 `nsh-*` crate；`mica-client` 有对应单测；`clients/mica_rust` 行数明显下降。

> **已完成（2026-11-24，按用户建议在 P0 验收后下沉）**：`f8a91686`。将零 UI 依赖的 `storage.rs`、`net.rs`、`ws_transport.rs`、`web_runtime.rs`、`web_idb.rs`（约 6,000 行、0 `nsh-*` 依赖）整体下沉至 `crates/mica-client/src/`，`clients/mica_rust` 保留薄重导出；`mica-client` 34/34 单测通过且 native/`wasm32` 双目标 0 告警。

---

### P2-2 `nsh` 依赖治理

- **背景**：`nsh` 是单一厂商（长桥 Longbridge）维护的 0.x 框架（当前 0.6.1，Apache-2.0）；客户端依赖树有 766 个 crate，包括 quickjs、wgpu、gtk4 等。
- **怎么做**：
  1. `clients/mica_rust/Cargo.toml` 中所有 `nsh-*` 用精确版本（`=0.6.1`），避免 `cargo update` 意外升级；
  2. 在仓库写一份 `nsh` 升级检查表：升级前读 release notes；升级后跑 P1-2 的输入法清单、P1-3 的端到端、W5 的体积门禁；
  3. 对 `nsh` 上游的已知问题（如 RichEditor 虚拟化 #2376、Firefox import map 问题）在文档里登记，便于跟进。

> **已完成（2026-11-24）**：`eb605e76`、`d71011a1`。`clients/mica_rust/Cargo.toml` 全部 `nsh-*` 精确锁定 `=0.6.1`；新增 [nsh-dependency-governance.md](../dev/nsh-dependency-governance.md) 登记升级检查表与 8 项上游 `nsh 0.6.1` 已知问题/绕过方案。

---

## 4. Web 首屏专项（P0-3，任务 W1–W6）

### 4.1 实测数据

**测量方法**：

- 构建：分支 `d5b0ccb`，`--profile web`，按 `scripts/build-web-rust.sh` 的方式运行 `wasm-bindgen`（加 `--remove-name-section --remove-producers-section`），再做 Brotli-11 预压缩；
- 服务：静态服务器按 `deploy/nginx.conf` 的规则返回预压缩的 `.br` 文件，带 `application/wasm` 类型和 ETag；
- 计时：headless Chromium（Playwright）加 CDP 限速，从导航开始计时到登录界面出现（`#status` 隐藏且 `#app` 有内容）；冷启动禁用缓存，每个条件跑两次；
- 脚本见 [assets/fullstack-rust-2026-11-24/](assets/fullstack-rust-2026-11-24/)，命令见 §8；
- 对照组是 #182 优化后的 Flutter Web 实测（[web-cold-start-2026-11-16.md](../performance/web-cold-start-2026-11-16.md)，同样用 Playwright + CDP 限速，计时到 Flutter 首帧）。

**包体积：**

| 文件 | 未压缩 | Brotli-11（实际传输） |
| --- | ---: | ---: |
| `mica_rust_bg.wasm` | 23,009,946 B | 5,267,896 B |
| `nsh.js` | 427,584 B | 104,144 B |
| `mica_rust.js` | 30,837 B | 5,184 B |
| **合计** | — | **约 5.13 MiB** |

> 只用 `cargo build`（未经 `wasm-bindgen` 去除名称段）时，wasm 为 25,516,160 B，Brotli 后 5,439,480 B，即此前报告的“5.19 MiB”。

**冷启动（首次访问）：**

| 网络（延迟 / 带宽） | Flutter Web（#182 实测） | 纯 Rust 版（本次实测） | 差距 |
| --- | ---: | ---: | ---: |
| 不限速 | — | 0.57–0.88 s | — |
| 120 ms / 25 Mbps | 约 1.85 s | 2.93–2.96 s | 慢约 1.1 s |
| 300 ms / 10 Mbps | 约 3.53 s | 6.74–6.79 s | 慢约 3.2 s |

**二次访问（浏览器已缓存，ETag 返回 304）：**

| 网络 | 纯 Rust 版 | 传输量 |
| --- | ---: | ---: |
| 120 ms / 25 Mbps | 0.93 s | 约 1 KiB |
| 300 ms / 10 Mbps | 1.80 s | 约 1 KiB |

> 字体：Flutter 版首帧之后还要下载中文字体切片（每片约 30 KB）；纯 Rust 版使用系统字体，不下载字体。

### 4.2 体积构成

未去除符号名的构建，用 twiggy 统计（未压缩的代码字节，用于比较各部分占比，不等于传输量）：

| 部分 | 体积 | 说明与能否缩减 |
| --- | ---: | --- |
| `nsh` 框架本身 | 7.09 MiB（占代码 37%） | 固定成本 |
| Rust 标准库与泛型展开 | 4.58 MiB | 部分会随其他模块移除而减少 |
| Mermaid（`merman` 等） | 2.13 MiB | **可按需加载**（W4） |
| 界面层 `mica-rust` | 0.82 MiB | — |
| `serde` | 0.69 MiB | — |
| `naga`（着色器翻译器） | 0.54 MiB | Mica 不用，**可关掉对应特性**（W3） |
| 正则（`regex` 0.40 + `fancy_regex` 0.15） | 0.55 MiB | 需查来源，可能随 Mermaid 一起移出 |
| 公式（`ratex`） | 0.37 MiB | **可按需加载**（W4） |
| Mica 内核（`mica-md`、`mica-board`、`yrs`、`mica-editor` 等） | 合计约 0.6 MiB | — |
| 其他（`markdown`、`lol_html`、`html5ever`、`pulldown_cmark`、`encoding_rs`、`brotli_decompressor`、`json5` 等） | 各 0.04–0.11 MiB | 需查来源，可能可去掉（W3） |
| 代码合计 | 19.38 MiB | — |
| `.rodata` 数据段 | 4.16 MiB | 未细分来源，W3 中一并排查 |

### 4.3 影响范围

- **桌面用户：不受影响**（原生程序，不下载这个包）。
- **Web 用户第一次打开**：比现在慢 1–3 秒，弱网更明显。
- **每次发版后**：wasm 文件名带内容哈希，所有 Web 用户都要重新下载一次。发版越频繁，“首次访问”越常出现。
- **分享页 `/s/`**：服务端生成，不受影响。
- **加载过程中**只有一行 “Loading Mica (Pure Rust WebAssembly)…” 文字（`clients/mica_rust/web/index.html`），没有进度，弱网下用户会以为卡住了。

### 4.4 优化任务

建议顺序：W5（先加门禁防止继续变大）→ W1 → W2 → W3 → W4 → W6。

#### W1 启动画面与预加载（约 1 天）

- **参照**：`main` 在 #182 做过同样的事，见 [web-cold-start-2026-11-16.md](../performance/web-cold-start-2026-11-16.md) 和 `main` 的 `clients/mica_flutter/web/index.html`。
- **怎么做**：
  1. `clients/mica_rust/web/index.html` 内联 Mica 标志（用内联 SVG，不用 `<img>`，否则图片请求会排在大文件后面），HTML 一到就显示；300 ms 后淡入，避免热启动闪一下；应用挂载后移除；
  2. 为 `nsh.js`、`mica_rust.js` 加 `modulepreload`，为 `mica_rust_bg.wasm` 加 `<link rel="preload" as="fetch" crossorigin>`；
  3. `scripts/build-web-rust.sh` 会把内容哈希写进 `index.html` 的资源 URL，预加载链接必须用同样的带哈希 URL；
  4. 预加载请求的模式和凭据要与 `nsh.js` 实际发出的 `fetch` 一致（`nsh.js` 约 1294–1296 行和 1403 行用 `compileStreaming(fetch(...))`），否则浏览器会下载两次（#182 踩过）；
  5. 改完 `index.html` 后同步更新 CSP 哈希（P1-5）。
- **验收**：限速 300 ms / 10 Mbps 下，HTML 到达后约 300 ms 出现标志；DevTools Network 中 wasm 只下载一次。

> **已完成（2026-11-24）**：`8c100db7`。`clients/mica_rust/web/index.html` 内联 Mica SVG 启动画面（300 ms 淡入），加入 `modulepreload` 与 `<link rel="preload" as="fetch" crossorigin>` 带哈希 URL；Playwright E2E `[1/6]` 实测 `mica_rust_bg.wasm` 仅下载 1 次。

#### W2 `wasm-opt -Oz`（约 0.5 天）

- **怎么做**：
  1. 在 `scripts/build-web-rust.sh` 的 `wasm-bindgen` 之后对 `mica_rust_bg.wasm` 运行 `wasm-opt -Oz`；
  2. binaryen 需要较新版本（建议 ≥ 116），并打开与 `rustc 1.100.0` 输出一致的特性（如 `--enable-bulk-memory --enable-reference-types --enable-multivalue --enable-sign-ext --enable-nontrapping-float-to-int`；校验失败时按报错补特性开关）；
  3. CI 和 Docker 构建固定 binaryen 版本（从 GitHub release 下载并校验 sha256）。
- **验收**：记录前后的未压缩和 Brotli 体积；Web 功能回归（登录、编辑、同步、公式、Mermaid、画板）正常。
- **注意**：本次评审试过用系统 apt 源里的 binaryen 108，测量中途被打断，没有结果；不要用这个旧版本。

> **已完成（2026-11-24）**：`8c100db7`。CI 与 `Dockerfile.web` 固定新版 binaryen，`scripts/build-web-rust.sh` 在 `wasm-bindgen` 后运行 `wasm-opt -Oz`（`mica_rust_bg.wasm` 未压缩由 `16.06 MB` 压至 `13.93 MB`），全套 Web E2E 回归通过。

#### W3 关掉 `nsh` 和依赖里用不到的部分（1–2 天）

- **怎么做**：
  1. 逐个查明来源，例如：
     `cargo tree --manifest-path clients/mica_rust/Cargo.toml --target wasm32-unknown-unknown -e features -i naga`
     对 `naga`、`markdown`、`pulldown_cmark`、`lol_html`、`html5ever` / `markup5ever`、`encoding_rs`、`brotli_decompressor`、`json5`、`fancy_regex`、`regex` 都查一遍；
  2. 能通过关闭特性去掉的就关掉。例如 `naga` 由 `nsh-core` 的 `shader` 特性引入，而 `nsh-core` 自身没有默认特性，说明是别的 `nsh` crate（如 `nsh-base` 或 `nsh-platform-web` 的某个特性）把它打开的；用上面的 `cargo tree -e features` 找到是谁打开，再在 `clients/mica_rust/Cargo.toml` 里对那个依赖关闭默认特性、只打开需要的；
  3. 去不掉但确实没用的，向 `nsh` 上游提 issue 或 PR，并登记到 P2-2 的文档。
- **验收**：用 `size-breakdown.py`（§8）确认对应 crate 从 wasm 中消失；Web 和桌面功能回归正常。

> **已完成（2026-11-24）**：`8c100db7`。收紧 `nsh-base` 默认特性与 Web 依赖特性集，结合 W2/W4 将首包压至 `3,926,345 B`（3.74 MiB），低于 4.0 MiB 切换门禁；Web 与桌面回归全绿。

#### W4 Mermaid 与公式改为按需加载（3–5 天）

- **背景**：`main` 本来就把它们拆成了按需加载的独立 wasm（`main` 的 `scripts/build-preview-wasm.sh`，产物 `mica_preview/{math,mermaid}.{js,wasm}`，普通页面零下载；Mermaid 包体积门禁 2 MiB gzip，公式 1 MiB gzip）。分支保留了这部分 crate 代码（`crates/md-wasm/src/preview.rs`，特性 `preview-math` / `preview-mermaid`），nginx 也还保留了 `location = /mica_preview_worker.js`，但删掉了构建脚本和加载逻辑，并通过 `mica-md` 的 `render` 特性把渲染器打进了主包。
- **怎么做**：
  1. `clients/mica_rust/Cargo.toml`：`mica-md` 的 `features = ["render"]` 只在桌面目标打开，`wasm32` 目标不打开；
  2. 恢复 `scripts/build-preview-wasm.sh`，输出目录改为 `deploy/web/mica_preview`，由 `scripts/build-web-rust.sh` 调用；`deploy/Dockerfile.web` 检查这些文件存在；
  3. 在 `clients/mica_rust/src/atomic_blocks.rs` 的公式 / Mermaid 渲染处，`wasm32` 下改为异步按需加载：通过 `wasm-bindgen` 的 extern 调用一个小 JS 加载器，首次用到时 `import('./mica_preview/<kind>.js')` 并渲染出 SVG（可参照 `main` 放进 Worker 执行的做法）；加载完成前显示源码或占位，结果按（类型、源码、主题）缓存；
  4. 桌面保持直接调用，不受影响。
- **验收**：
  - `size-breakdown.py` 中主包不再出现 `merman`、`ratex`；
  - 不含公式和 Mermaid 的页面，Network 面板里没有 `mica_preview` 请求；
  - 含公式和 Mermaid 的页面能正确渲染，失败时安全降级显示源码；
  - 按需包体积不超过 `main` 原有门禁。

> **已完成（2026-11-24）**：`8c100db7`。Web 目标下 `mica-md` 仅启用 `render-board`（剔除 `merman` / `ratex`）；恢复 `scripts/build-preview-wasm.sh` 输出 `mica_preview/{math,mermaid}.{js,wasm}`（实测 gzip-9：`math` `1,003,574 B` ≤ 1 MiB、`mermaid` `2,010,015 B` ≤ 2 MiB）；`atomic_blocks.rs` 经根 `Owner` + `nsh_core::prelude::spawn` 异步按需懒加载并缓存，Playwright E2E `[6/6]` 验证普通页面零请求、公式页按需拉取渲染通过。

#### W5 体积门禁（约 0.5 天，建议最先做）

- **怎么做**：
  1. `scripts/build-web-rust.sh` 末尾对 `mica_rust_bg.wasm`、`mica_rust.js`、`nsh.js` 分别做 Brotli-11 压缩后求和；超过预算就失败，报错写明实际值和预算（CI 需安装 `brotli` 命令）；
  2. 预算分两步：
     - **立即**设为 5,500,000 B，防止继续变大；
     - **最终门禁 4,194,304 B（4.0 MiB）**，用户已确认；W2–W4 完成后若实测更小，再收紧到“实测值 + 5% 余量”；
     - 达不到 4.0 MiB 不能切换（§5.1），需要继续裁剪（例如把画板、导入导出也改为按需加载）；
  3. 调高预算须附实测数据和理由，做法同 [wasm 体积预算记录](wasm-size-budget-2026-11-17.md)。
- **验收**：CI 中门禁生效；人为加一个大依赖时构建失败。

> **已完成（2026-11-24）**：`8c100db7`。`scripts/build-web-rust.sh` 已启用 Brotli-11 硬门禁 `≤ 4,194,304 B`（4.0 MiB），[CI 45377675471](https://github.com/weironz/mica/actions/runs/45377675471) 实测 **`3,926,345 B`（3.74 MiB）**通过。

#### W6 发版后的重复下载（说明，不单独开发）

- 每次发版 wasm 内容都会变，所有 Web 用户需要重新下载一次主包。W2–W4 把主包压小就是最直接的缓解。
- 发版节奏上，可以考虑 Web 改动攒一批再发，减少用户重复下载。
- Service Worker 后台预取新版本这类方案，等前面几项做完再评估。

### 4.5 目标

| 阶段 | 首包（Brotli） | 25 Mbps / 120 ms 首屏 | 说明 |
| --- | ---: | ---: | --- |
| 现状 | 约 5.13 MiB | 约 2.95 s | 本次实测 |
| W2–W4 完成后（估算） | 约 3.7–4.2 MiB | 约 2.3–2.5 s | 估算依据：去掉 Mermaid、公式、`naga` 等的代码占比，加 `wasm-opt` 常见的 5–15% 收益；需实测确认 |
| **切换门禁（用户已确认）** | **≤ 4.0 MiB** | — | 估算区间上沿超过门禁，若 W2–W4 后仍超出，需继续裁剪 |
| 对照：现行 Flutter Web | 约 2.9 MiB + 字体切片 | 约 1.85 s | #182 实测 |

> `nsh` 框架本身约 7 MiB 未压缩代码是固定成本，首包很难压到比 Flutter 版更小；目标是“接近”，加上启动画面后体感差距会进一步缩小。

---

## 5. 正式切换步骤与闸门

### 5.1 闸门（全部满足才进入 C1）

- [x] P0-1 ～ P0-4 全部完成并按验收标准验证（`0804738f`、`03981b66`、`8c100db7`、`86fa49fc`、`dcd985a6`；注：P0-4 线上真实 X → X+1 双版本自动更新在首个切换版发布后补验，本地校验/覆盖安装/单实例/回滚已在 Windows 物理机全链路通过）
- [x] P1-1 文档、发布说明、对外平台列表已按“不提供 Android 和 macOS 桌面”更新（`eb605e76`）
- [ ] P1-2 Windows 输入法与三种浏览器的验证报告已归档，全部通过（报告已归档于 [fullstack-rust-ime-browser-verification-2026-11-24.md](fullstack-rust-ime-browser-verification-2026-11-24.md)，用户 Windows 桌面 + Chrome 真人实测已通过；**未测项**：Windows Firefox、搜狗拼音、macOS Safari，待用户确认是否补测或直接放行并在发版说明标注）
- [x] P1-3 测试补齐：按用户 2026-11-24 务实调整（重高价值回归与核心 E2E，不凑 200 条水测试），`mica-editor` 已扩至 73 条行为回归 + `proptest`，Web 6 场景 Playwright E2E 已进 CI，回执下限已全部上调（`3bdb9a9a`、`6daa44f7`、`23bfecfa`、`48c5d219`）
- [x] P1-4 完整 CI 与 Release 彩排全绿（[CI 45377675471](https://github.com/weironz/mica/actions/runs/45377675471) 7/7、[Release dry-run 45374949409](https://github.com/weironz/mica/actions/runs/45374949409) 9/9；合并 `main` 前针对近期 4 笔体验修复 `dcd985a6..48c5d219` 补跑终版全量 CI）
- [x] P1-5 CSP 恢复按哈希放行（`8c100db7`，`test-web-csp-hashes.sh` + Playwright E2E 0 违规）
- [x] W5 体积门禁 ≤ 4,194,304 B（Brotli）在 CI 中生效并通过（`8c100db7`，CI 实测 **`3,926,345 B` / 3.74 MiB**）
- [ ] A1b 已通过评审和完整 CI，待切换时合入（完整 CI 含真实 Postgres 迁移 `0040` 已绿；待对 `#191` 口径、隐私与写入热路径做一次最终代码评审确认）
- [ ] M4 切换前冒烟全部通过（桌面/Web 核心编辑、同步、升级与回滚已实测通过；待终版全量 CI 绿灯与切换前最终冒烟确认）
- [x] 回滚预案已演练（见 C1、C2：C1 桌面卸载 + 重装官方 Flutter `v0.14.11` + 124 MB `store.db` `PRAGMA quick_check = ok` 已于 2026-11-24 物理机演练通过；C2 Web 切换部署前保留最后一个 Flutter Web 镜像 tag）

### 5.2 切换步骤

- **C0 服务端**：A1b 合入 `main`，与新客户端在同一个版本里发布（带迁移部署须先有经内容校验的还原点，见 `AGENTS.md`）。
- **C1 桌面**：
  - 正式安装器沿用原 AppId，带 `[InstallDelete]` 清理 Flutter 文件（P0-4）；
  - 新版首次启动时检查并推送旧缓存里未推送的编辑（P0-1）；
  - 通过 Flutter 版的自动更新把用户升级过来；
  - 保留 0.14.x Flutter 安装包可下载，作为回滚途径。
- **C2 Web**：
  - 主域名切到新客户端；
  - 保留 Flutter Web 镜像的最后一个 tag，必要时用上一镜像回滚部署；
  - 切换后观察首屏时间和错误上报。
- **C3 退役**：确认不需要回滚后，删除 `clients/mica_flutter`、`crates/doc-wasm`、`crates/board-wasm`、Flutter 相关脚本与 CI（可参照分支提交 `aee801f` 的删除范围），并更新 `AGENTS.md` 中与 Flutter / Dart 相关的规则。

---

## 6. 评审中确认做得好、应保留的部分

- `mica-editor` 的 `EditorSession` 直接持有 `MicaDoc` 做 CRDT 事务，没有引入 ProseMirror 式第二套文档表示（守住红线 1）。
- `mica-sync` 不直接做网络和存储 I/O，同步状态机可以确定性测试；桌面端从本地存储回放未确认编辑。
- 本地库改动只加列和新表，没有升 `SCHEMA_VERSION`，保留了回退到 0.14.11 的能力。
- 服务端改动（#191 使用统计、`/metrics`、迁移 `0040`）全部是增量的，老客户端不受影响。
- 依赖审计覆盖了客户端 lock 文件（`scripts/check-rust-audit.sh`）。
- Web 端凭据改为 HttpOnly Cookie，不再存进 localStorage。
- 冷启动 0.6–0.9 秒（不限速）说明编译和执行本身不慢，问题集中在传输体积，方向明确。

---

## 7. 用户已决定的事项（2026-11-24）

| # | 事项 | 决定 | 落到哪里 |
| --- | --- | --- | --- |
| 1 | 正式切换时是否提供 Android、macOS 桌面 | **不提供** | P1-1、P1-2、§5.1 |
| 2 | Web 首包体积门禁最终数值 | **≤ 4.0 MiB（4,194,304 B，Brotli）** | W5、§4.5、§5.1 |
| 3 | 预览渠道观察期 | **不设观察期** | §0.2、M4、§5.1 |
| 4 | #191 使用统计服务端是否先发 | **不先发**，随切换版本一起发布 | A1b、C0 |

---

## 8. 附录：复现方法

测量脚本在 [assets/fullstack-rust-2026-11-24/](assets/fullstack-rust-2026-11-24/)：

| 文件 | 作用 |
| --- | --- |
| `serve.js` | 静态服务器，按 `deploy/nginx.conf` 的规则返回 `.br` 预压缩文件、`application/wasm` 类型和 ETag |
| `cold.js` | 冷启动计时（禁用缓存，CDP 限速），可截图 |
| `warm.js` | 二次访问计时（先加载一次建立缓存，再限速重载） |
| `size-breakdown.py` | 按 crate 汇总 twiggy 输出的代码体积 |

```bash
# 1. 构建 Web 包（产物在 deploy/web/）
bash scripts/build-web-rust.sh
python3 -c "
import brotli
for f in ['mica_rust_bg.wasm', 'mica_rust.js', 'nsh.js']:
    d = open('deploy/web/' + f, 'rb').read()
    open('deploy/web/' + f + '.br', 'wb').write(brotli.compress(d, quality=11))
"   # 需要 pip install brotli；Docker 镜像构建时会自动生成 .br

# 2. 计时（需要 Node 与 Playwright 的 Chromium）
A=docs/reviews/assets/fullstack-rust-2026-11-24
node $A/serve.js "$PWD/deploy/web" 8099 &
node $A/cold.js http://127.0.0.1:8099/ 120 25 cold.png
node $A/cold.js http://127.0.0.1:8099/ 300 10
node $A/warm.js http://127.0.0.1:8099/ 120 25
node $A/warm.js http://127.0.0.1:8099/ 300 10

# 3. 体积构成（保留符号名构建）
CARGO_PROFILE_WEB_STRIP=false cargo build --manifest-path clients/mica_rust/Cargo.toml \
  --lib --target wasm32-unknown-unknown --profile web --locked
twiggy top -n 100000 --format csv \
  clients/mica_rust/target/wasm32-unknown-unknown/web/mica_rust.wasm > twiggy.csv
python3 -I $A/size-breakdown.py twiggy.csv
```

每完成一项任务，在本文对应条目下追加“已完成：提交号 + 验证方式 + 实测数据”，并勾选 §5.1。
