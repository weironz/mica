# 全栈 Rust 分支评审与整改清单（2026-11-24）

> 评审对象：分支 `feat/fullstack-rust-architecture` @ `d5b0ccb`（基于 `main` @ `8b07d76`，58 个提交，1,801 个文件，+87,383 / −183,970 行）
> 对照文档：该分支的 [fullstack-rust-architecture.md](https://github.com/weironz/mica/blob/feat/fullstack-rust-architecture/docs/architecture/fullstack-rust-architecture.md)、[P0 实证报告](https://github.com/weironz/mica/blob/feat/fullstack-rust-architecture/docs/reviews/fullstack-rust-p0-spike-2026-11-22.md)
> 用途：交给实施者逐条整改；每条都写了证据、影响、修复要求和验收标准。文中 `文件:行号` 均指分支 `d5b0ccb`。

---

## 0. 结论与最终建议

**方向保留，发布方式改掉。**

- **保留**：把编辑器、同步、画板交互收进零 UI 依赖的纯 Rust crate（L1/L2：`mica-editor`、`mica-sync`、`mica-board::session`、`mica-client`）。这是对的，和现有“单一实现”原则一致。
- **不要**：把这个分支整体合进 `main` 并发布生产。它在一次合并里删掉了正在生产使用的 Flutter 客户端，换成基于 0.x 框架 `nsh` 的新客户端；而分支自己的架构文档定下的红线 4（“拒绝大爆炸重写、小步合入”）和 Phase 3 三道启动闸门都没有满足，评审中还确认了几处会直接伤到现有用户的回归（§3 的 P0 项）。

**推荐路线（三步）：**

1. **拆分合入**：L1/L2 crate 与服务端改动单独成 PR，过完整 CI 后合进 `main`；服务端可以照常发版。Flutter 继续做生产客户端。（§2 阶段 A）
2. **预览渠道并行**：新客户端以独立身份（独立 AppId / 安装目录 / Web 子路径）作为“Mica Preview”灰度，和 Flutter 并存，修完 §3 的 P0、P1 项。（§2 阶段 B）
3. **满足闸门再切换**：§5 的切换闸门全部打勾后，再把正式发布链路切到新客户端、退役 Flutter。（§2 阶段 C）

---

## 1. 评审范围与方法

**做了的：**

- 逐项阅读分支的架构文档、P0 报告、`ui-gpui-evaluation.md` §10，以及新客户端 `clients/mica_rust`、L2 crate、服务端、CI / 发布 / 部署配置的改动；
- 与 `main` 的对应实现逐项对照（缓存键、Web 存储、安装器、自动更新、发布产物、CSP）；
- 在 Linux 上用 `rustc 1.100.0` 实际构建分支的 Web 包（`cargo build --manifest-path clients/mica_rust/Cargo.toml --lib --target wasm32-unknown-unknown --profile web --locked`），测量体积；
- 查询该分支的 GitHub Actions 记录。

**没做的（需要实施方补）：**

- 没有运行桌面 GUI、没有真人输入法测试、没有真机；
- 没有在该分支跑完整 CI（该分支 **从未有过 Actions 运行**，见 P1-4）。

---

## 2. 推荐的合入与发布路线

### 阶段 A：拆分合入（1 周内可完成）

| PR | 内容 | 合入条件 |
| --- | --- | --- |
| A1 | 服务端与共享 crate：`mica-core`（`word_count.rs`、`usage_stats.rs`、`store_stats.rs`、本地库增量列）、`app-core` / `api-server`（#191 使用统计、`/metrics` 扩展、迁移 `0040_usage_stats.sql`）、`interchange`（画板 SVG） | 完整 `just ci`（含真实 Postgres）通过；迁移只增不改（已满足：`0040` 只加列和新表） |
| A2 | L2 crate：`mica-editor`、`mica-sync`、`mica-board::session`、`mica-client` | 进根 workspace，`cargo test --workspace` 通过；不改现有客户端行为 |
| A3 | 新客户端 `clients/mica_rust` 作为**新增**目录合入（不删除 `clients/mica_flutter`），CI 增加独立 job | 不影响现有发布链路 |

> A1 可单独发版：服务端改动全部是增量的，老 Flutter 客户端不受影响。

### 阶段 B：预览渠道并行（修完 P0 / P1）

- Windows 安装器用**新的 AppId 和安装目录**（如 `Mica Preview`），与 Flutter 版共存，互不覆盖；
- Web 新客户端挂在独立路径（如 `/next/`），主域名继续服务 Flutter Web；
- 预览版对同一账号读写同一服务器，是天然的“对拍”环境。

### 阶段 C：正式切换

- §5 闸门全部打勾后，正式发布链路切到新客户端，执行 §3 P0-4 的升级与回滚方案，再删除 Flutter 及其脚本。

---

## 3. 整改清单

优先级：**P0 = 发生产前必须修（阻断）**；**P1 = 切换前必须完成**；**P2 = 建议，影响长期可维护性**。

### P0-1 云端缓存键退回“只按服务器”，#111 的账号隔离失效

- **证据**：
  - `main` 的云端缓存键是 `[规范化服务器地址, 账号 id]` 的 JSON 元组（`clients/mica_flutter/lib/local/cache_stats.dart:101` `cloudCacheOrigin`），#111 起所有缓存读写都经过它；旧版“只按服务器地址”的行被刻意隔离，不认领、不删除。
  - 分支新客户端直接用服务器地址做 origin：`clients/mica_rust/src/storage.rs:1895` `normalize_origin` 只做 `trim` 和去尾斜杠；`clients/mica_rust/src/lib.rs:1097`、`clients/mica_rust/src/cloud_sync.rs:415` 把这个地址传给 `open_cloud_doc`，`clients/mica_rust/src/doc_sync_session.rs:149` 用它建本地同步存储。
- **影响**：
  1. 老用户在 Flutter 版里**还没推送的离线编辑**存在元组键下，新客户端看不到，用户感知为“编辑丢了”；
  2. 同一台机器、同一服务器的两个账号共用一份缓存（#111 修过的隐私问题重新出现）；
  3. #111 隔离的“无法确认属于哪个账号”的旧缓存，会被新客户端当成当前账号的数据读写。
- **修复要求**：
  - 在 L2（`mica-client`）实现与 `main` 等价的 `cloud_cache_origin(server, account_id)`：规范化规则与 `normalizeServerUrl` 一致（scheme/host 小写、去默认端口、去尾斜杠），账号 id 区分大小写，空账号直接报错；
  - 所有云端缓存读写只经过这个函数；
  - 首次启动时认领当前账号的元组键数据，并把其中未推送的 outbox 推上服务器；只按服务器地址的旧行继续隔离。
- **验收**：
  - 移植 `main` 的 `cache_stats` / `cloudCacheOrigin` 相关测试到 Rust，全部通过；
  - 端到端：Flutter 0.14.11 离线编辑 → 不联网退出 → 安装新客户端 → 登录同一账号 → 编辑出现并成功推送；
  - 同机先后登录 A、B 两个账号，互相看不到对方页面。

### P0-2 Web：主线程同步请求，且没有离线持久化

- **证据**：
  - `clients/mica_rust/src/web_runtime.rs:81` 用 `open_with_async(method, url, false)` 发**同步** XHR，并且按规范不能设超时；所有云端 HTTP 调用都走这里。
  - Web 端文档只存在内存：`clients/mica_rust/src/storage.rs:174` `MemoryVault`、`:596` `MemoryMirrorStorage`。`main` 的 Web 有 IndexedDB 持久化未发送编辑（`clients/mica_flutter/lib/cloud/web_idb_doc_store.dart`）。
- **影响**：
  - 网络慢或服务端无响应时，整个页面冻结（无法滚动、无法输入），且无法超时恢复；
  - 断网时编辑只在内存里，刷新或关闭标签页即丢失。
- **修复要求**：
  - HTTP 改为异步 `fetch`（`wasm-bindgen-futures`），带超时、取消和会话代际校验（对照 [异步评审清单](../dev/async-review-checklist.md)）；
  - 为 `mica-sync` 的 `SyncStorage` 增加 IndexedDB 实现（这本来就是架构文档 Phase 1 M1.2 的计划），未发送编辑落盘；多标签页互斥沿用 `main` 的 Web Locks 做法和 #161 的教训。
- **验收**：
  - Chrome DevTools 限速到 Slow 3G、以及服务端挂起 30 秒两种情况下，界面保持可操作，请求超时后有错误提示；
  - 断网编辑 → 刷新页面 → 编辑仍在 → 恢复网络后自动推送成功；
  - 两个标签页同时编辑同一文档不丢编辑。

### P0-3 Web 首次加载体积约为现行版的 1.8 倍，且没有体积门禁

- **实测**（分支 `d5b0ccb`，`--profile web`，未经 `wasm-opt`，构建脚本本身也没有运行 `wasm-opt`）：

  | 产物 | 未压缩 | gzip -9 | Brotli -11（实际传输） |
  | --- | ---: | ---: | ---: |
  | `mica_rust.wasm` | 25,516,160 B | 7,601,427 B | **5,439,480 B（5.19 MiB）** |
  | `nsh.js` | 427,584 B | 125,197 B | — |
  | 对照：现行 Flutter Web（P0 报告数据） | — | — | 约 2,983 KiB |
  | 对照：P0 报告对纯 Rust 方案的预估 | — | — | 约 1.85 MiB |

- **原因**：公式（`ratex`）、Mermaid（`merman`）、画板等原本按需加载的部分都打进了主包；`scripts/build-web-rust.sh` 不检查体积；`index.html` 只有一行文字的加载提示。
- **影响**：#182 刚解决的“首次打开很慢”会明显变差；低端设备上 25 MB wasm 的编译耗时和内存也更高。
- **修复要求**：
  - 构建加 `wasm-opt -Oz`；
  - 公式、Mermaid、画板、导入导出拆成按需加载的独立模块（参照 `main` 的按需预览包做法）；
  - `build-web-rust.sh` 加硬体积门禁（建议以现行 Flutter Web 的约 2,983 KiB Brotli 为上限，数值由用户拍板，参照 [wasm 体积预算记录](wasm-size-budget-2026-11-17.md) 的做法写明理由）；
  - 加载页恢复 #182 的启动画面与预加载。
- **验收**：门禁进 CI；无痕窗口冷启动到可编辑的时间不劣于现行 Flutter Web（在生产同等网络下实测并记录）。

### P0-4 桌面自动更新是假的；从 Flutter 原地升级会留下旧程序

- **证据**：
  - “检查更新”只设置一句固定文案：`clients/mica_rust/src/lib.rs:2884`、`:2911`、`:3978` 都是 `update_status.set(format!("已是最新版本（v{APP_VERSION}）"))`，没有任何网络请求。
  - 新安装器沿用 Flutter 版的 AppId（`clients/mica_rust/installer/mica.iss:14`），程序名从 `mica_flutter.exe` 变为 `mica-rust.exe`，但没有 `[InstallDelete]` 清理旧文件。
- **影响**：
  - 老 Flutter 版的更新器会把用户升级一次到新客户端，之后就**再也无法推送修复**；
  - 安装目录里残留 `mica_flutter.exe` 及其运行库，用户固定在任务栏的旧图标仍会启动旧程序，新旧两个程序读写同一个本地库。
- **修复要求**：
  - 实现真正的更新检查与安装：与 Flutter 版 `DesktopUpdateController` 行为一致（查询最新 release、比较版本、下载安装包、用 `sha256-windows-installer.txt` 校验、静默安装后重启）；
  - 安装器 `[InstallDelete]` 删除 Flutter 版遗留文件；
  - 新旧程序使用同一个单实例互斥量，防止同时运行。
- **验收**：
  - 发布 X 与 X+1 两个版本，X 版内“检查更新”能发现并安装 X+1；
  - 在装有 0.14.11 Flutter 版的机器上升级，安装目录不再有 `mica_flutter.exe`，原任务栏固定项失效或指向新程序；
  - 回滚：卸载后重装 0.14.11 能打开原本地库（本地库只加列未升版本，这一点分支已经做对，需保留）。

### P1-1 平台覆盖缩水，需要明确决策

- **证据**：分支的 `.github/workflows/release.yml` 删除了 `main` 的 Android APK 任务；macOS 只有 CLI，没有桌面安装包。架构文档红线 3 写明“移动端触屏编辑能力不裁撤”。
- **修复要求**：二选一并写进架构文档：恢复 Android（和 macOS 桌面）产物；或者由用户明确批准“暂时不提供”，并在发布说明中告知现有 Android 用户。

### P1-2 中文输入法与浏览器、真机未验证

- **证据**：P0 报告 §2.4 自列为“剩余外设补验项”：Windows 真人微软拼音/搜狗、Android 真机、Firefox / Safari。后续文档没有补验记录。架构文档 Phase 3 闸门 2 要求这些全部通过。
- **修复要求**：按架构文档 P0.1 的验收项做真人测试，结果写入 `docs/reviews/`：拼音连续组字、候选窗跟随光标、Esc 取消不留残字、退格不误删正文、中英快速混打不重复上屏、复制粘贴。范围是 Windows（微软拼音、搜狗）、macOS 拼音、Chrome / Firefox / Safari、Android 真机。
- **验收**：报告每项附截图或录屏，不能用 CDP 或合成事件替代。

### P1-3 测试覆盖大幅下降

- **证据**：

  | | `main` | 分支 |
  | --- | --- | --- |
  | 客户端测试 | 331 个 Flutter 测试文件（2,200+ 用例）+ 4 套浏览器端到端 + Windows 集成测试 | `clients/mica_rust` 142 个 `#[test]`；CI 回执下限只有 15（`.github/workflows/ci.yml:174`） |
  | 编辑器内核 | Flutter 编辑器回归用例 | `mica-editor` 26 个测试；架构文档 M2.2 要求 ≥200 条并含属性测试 |
  | 新 Web 客户端浏览器端到端 | — | 没有 |

- **修复要求**：
  - 把 `main` 中编辑器行为类回归用例（marks、selection、表格、列表、粘贴、撤销、输入规则、IME 组字）系统性移植到 `mica-editor`，达到计划的 ≥200 条；
  - 新 Web 客户端补浏览器端到端：登录、打开文档、编辑、实时同步、断网恢复（Playwright 驱动 `nsh` 的 DOM）；
  - CI 回执下限改为实际用例数附近，防止静默少跑。

### P1-4 分支从未跑过 CI，Linux 依赖可能不全

- **证据**：该分支 GitHub Actions 运行记录为 0。新客户端依赖 `gtk4 0.11`（经 `nsh-platform-linux`），而 CI 安装的是 `libgtk-3-dev`（`.github/workflows/ci.yml:163`），可能导致 Linux 构建失败（未实测）。
- **修复要求**：在分支上手动触发一次完整 `CI`（scope=full）和 `Release` 彩排，修到全绿；补齐 Linux 依赖。

### P1-5 Web CSP 退化为 `'unsafe-inline'`

- **证据**：`deploy/nginx.conf:16` 的 `script-src` 从 `main` 的按内容哈希放行改为 `'unsafe-inline'`；`scripts/test-web-csp-hashes.sh` 被删除。
- **修复要求**：为 `index.html` 的内联模块脚本计算哈希，恢复哈希放行和对应测试。

### P2-1 把界面层里的业务逻辑下沉到 L2（关系到“可换壳”是否成立）

- **证据**：架构文档要求 L3 是“≤20% 的薄壳”，以保证“未来可只换 L3”。实际 `clients/mica_rust` 有约 5.3 万行；其中下列文件几乎不依赖 `nsh`，属于 L2 职责：

  | 文件 | 行数 | `nsh` 引用数 |
  | --- | ---: | ---: |
  | `storage.rs` | 2,507 | 0 |
  | `net.rs` | 1,811 | 0 |
  | `ws_transport.rs` | 586 | 0 |
  | `cloud_sync.rs` | 1,456 | 1 |
  | `doc_sync_session.rs` | 821 | 2 |

  而 `crates/mica-client` 只有 981 行。
- **修复要求**：把存储、HTTP/WS 传输、云端同步编排、导入导出的非界面部分移入 `mica-client`，`clients/mica_rust` 只保留视图与平台胶水。
- **验收**：上述模块迁出后不依赖 `nsh-*`；`mica-client` 有对应单测；`clients/mica_rust` 行数明显下降。

### P2-2 重新评估 Web 端的界面框架

- `ui-gpui-evaluation.md` §10 自己的分析是：同类产品（Notion / AFFiNE / Linear / Obsidian）在“桌面与 Web 同等重要 + 富文本 + 白板”约束下普遍用 DOM 做 Web 壳。
- `nsh` 的 Web 端目前体积（P0-3）和离线能力（P0-2）都未达标。
- 建议：短期 Web 继续用 Flutter 版或预览路径；做完 P2-1 后，评估以 DOM 系框架（如 Dioxus Web）做 Web 壳、复用同一套 L2。这项决策由用户拍板。

### P2-3 框架依赖治理

- `nsh` 是单一厂商维护的 0.x 框架（0.6.1，Apache-2.0）。客户端依赖树有 766 个 crate，包括 quickjs、wgpu、gtk4 等。
- 建议：
  - `Cargo.toml` 中 `nsh-*` 用精确版本（`=0.6.1`）；
  - 每次升级 `nsh` 走一次完整回归（P1-2 的输入法清单 + P1-3 的端到端）；
  - 在仓库记录升级检查表。

---

## 4. 评审中确认做得好、应保留的部分

- `mica-editor` 的 `EditorSession` 直接持有 `MicaDoc` 做 CRDT 事务，没有引入 ProseMirror 式第二套文档表示（守住红线 1）。
- `mica-sync` 不直接做网络和存储 I/O，同步状态机可以确定性测试；桌面端从本地存储回放未确认编辑（`crates/mica-sync/src/engine.rs` 的 `outbox_after`）。
- 本地库改动只加列和新表，没有升 `SCHEMA_VERSION`，保留了回退到 0.14.11 的能力。
- 服务端改动（#191 使用统计、`/metrics`、迁移 `0040`）全部是增量的，老客户端不受影响。
- 依赖审计覆盖了客户端 lock 文件（`scripts/check-rust-audit.sh`）。

---

## 5. 切换闸门（全部满足才允许阶段 C）

- [ ] P0-1 ～ P0-4 全部修复并按验收标准验证
- [ ] P1-1 平台范围有用户书面决策
- [ ] P1-2 真人输入法 / 浏览器 / 真机报告归档，全部通过
- [ ] P1-3 `mica-editor` ≥200 条测试；新 Web 有浏览器端到端；CI 回执下限已上调
- [ ] P1-4 分支完整 CI 与 Release 彩排全绿
- [ ] P1-5 CSP 恢复哈希放行
- [ ] 预览渠道（阶段 B）至少运行两周，无数据丢失类问题
- [ ] 回滚预案演练：保留 0.14.x Flutter 安装包与 Web 镜像，可在一次发版内切回

---

## 6. 实施建议

- 按 §2 的 PR 顺序推进，每个 PR 单独过完整 CI 合入 `main`，不再在长分支上积压。
- P0-1、P0-4 涉及老用户数据和升级通道，建议先做，并在 Windows 真机上验证。
- 每完成一项，在本文对应条目下追加“已修复：提交号 + 验证方式”，并勾选 §5。
