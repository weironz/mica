# 14 — 工作区管理、本地/云端世界切换、同步与公开分享

> **适用范围**：桌面端、Web、移动端。

## 核心不变量

1. **本地与云端按源（Origin）严格隔离**：本地离线 SQLite Vault 与多个不同云端服务器（`origin`）的数据、凭据、工作区与同步队列按 `origin` 完全隔离，切换世界绝不串数据或泄漏凭据。
2. **异步网络响应必验归属**：登录、拉取树、同步文档或获取成员列表跨 `await` 返回时，必须核对当前活动 `origin`/账号/会话代际；切换世界后的在途旧响应一律丢弃。
3. **桌面端无账号可离线使用，Web 端纯云端无本地工作区**：桌面端支持零注册、零联网的本地离线 SQLite Vault（默认无缝兼容旧版 Flutter `store.db` 路径并保留早期纯 Rust `vault.sqlite` 回退与 `MICA_DATA_DIR` 覆盖）；Web 端没有持久本地 SQLite 库，严禁启动或回退到假「本地工作区」，直接绑定当前 `window.location.origin` 作为纯云端客户端，未登录时展示不可关闭的全屏云端登录门禁；Web 鉴权走服务端 `HttpOnly` Cookie（`mica_token` + `mica_refresh`）与 `POST /api/auth/refresh` / `POST /api/auth/logout`，`window.localStorage` 仅保存非敏感 UI 偏好、严禁落盘任何 Token。
4. **云端文档实时 CRDT 协同优先，禁止并发整篇覆盖**：已打开的云端文档默认建立 `/ws/workspaces/{ws}/documents/{doc}?v=1` WebSocket 连接并由 `crates/mica-sync` `SyncEngine` 驱动增量 yrs diff 同步（同时订阅 `/ws/workspaces/{ws}/views` 热刷新目录树）；WS 协同激活期间跳过 HTTP `PATCH /markdown`（`replace_all`），避免并发编辑互相覆盖。
5. **单飞令牌轮换与绝不展示假空白页**：客户端所有云端 HTTP 401 刷新与 WebSocket 401 重连刷新必须经由按 `(server_url, refresh_token)` 归并的单飞（single-flight）协调器（含 RAII 取消唤醒与短时轮换结果复用缓存），避免并发请求重放同一旧 `refresh_token` 触发服务端重放检测并吊销整条登录族；一旦云端会话真正过期，主界面必须展示醒目的重新登录横幅，且未在本地缓存的云端页面必须展示「重新登录 / 重试」遮罩卡（严禁静默渲染成假空白文档）。

---

## 能力与交互规格

### 世界切换与多服务器账户
- **`CAP-14-01` 底部账号菜单与堆叠卡片浮层**：侧栏底部常驻当前身份行（桌面端本地模式显示「本 / 本地工作区 / 这台设备」；云端显示用户头像与邮箱），点击弹出堆叠卡片菜单（身份卡 + 设置/使用统计/[桌面端]切换世界/检查更新 + 退出登录或登录云端）。Web 端不展示「切换世界…」与任何本地模式文案，点击「退出登录」清除当前 `origin` 凭据并立即回到不可关闭的全屏云端登录页。
- **`CAP-14-02` 分屏世界切换与多服务器管理**：全窗口分屏「切换世界」界面（宽屏 `>=900px` 左深色径向渐变主视觉 + 右 `420px` 操作面板）。桌面端支持「云端账户 / 本地模式」切换、多服务器列表持久化、自动 `/api/health` 健康探测与重试、免密切换已登录服务器、添加/删除服务器、登录/注册/忘记密码表单；Web 端隐藏「云端账户 / 本地模式」切换页签与多服务器下拉管理，仅展示当前同源服务器健康状态与登录/注册/忘记密码表单，未登录时隐藏「返回」关闭按钮。

### 工作区管理、跨工作区迁移与成员邀请
- **`CAP-14-03` 工作区下拉切换菜单**：`320px` 宽工作区菜单（超过 8 个工作区自动出现搜索过滤框）、显示页面计数、每行 `⋯` 菜单支持重命名、导出（Mica 备份 / Markdown ZIP）、导入（文件/文件夹）、删除；底部提供「+ 新建工作区」与「导入工作区（ZIP / Notion）」。
- **`CAP-14-04` 跨工作区移动与复制**：支持将单页、文件夹子树或目录树多选批量项通过目标树选择器移动/复制到另一工作区指定目录。
- **`CAP-14-05` 工作区设置与成员邀请**：展示角色、工作区 ID、存储用量、重命名与成员管理；添加成员走「发送邀请 → 对方接受后加入」契约（见 `docs/features/workspace-invitations.md`）。

### 公开分享与 API 令牌
- **`CAP-14-06` 公开分享（Publish-to-Web）**：云端页面提供「共享」按钮与弹窗，支持开启/关闭公开访问、生成并一键复制 `/s/{token}` 公开只读链接（绿勾反馈）。
- **`CAP-14-07` API 令牌管理**：在设置页「账户 → API 令牌」支持创建、展示一次性令牌与吊销，供 CLI (`mica-cli`) 与 MCP Server (`mica-mcp`) 访问。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-14-01` / `02` 账号菜单与多服务器世界切换 | P0 | 桌面/Web | ✅ | `world_switcher::tests` + `net::tests` + `app_flows` |
| `CAP-14-03` / `04` 工作区菜单与跨工作区移动/复制 | P0 | 全端 | ✅ | `workspace_views::tests` + `transfer_dialog::tests` + `storage::tests` |
| `CAP-14-05` 工作区设置与成员邀请 | P1 | 桌面/Web | 🟡 | UI 与 `net::tests` 本地 mock 已通，待实连线上 Mica Server 联调 |
| `CAP-14-06` / `07` 公开分享与 API 令牌 | P1 | 桌面/Web | 🟡 | `share_dialog::tests` + `net::tests` 已通，待线上联调 |

---

## 演进记录

- **2026-11-23**：沉淀本地/云端按源隔离、世界切换、工作区治理与公开分享规格。
- **2026-11-23**：在纯 Rust 客户端补齐云端登录后的工作区列表拉取（`GET /api/workspaces`）、按工作区隔离的视图树镜像（`GET /api/workspaces/{ws}/views`，兼容服务端 `object_type="document"`）、文档 Markdown 读写与 401 自动 refresh token 轮换，以及启动自动恢复与登出回切本地。
- **2026-11-24**：移除 Web 端（`wasm32`）的本地工作区与本地模式入口；新增 `web_runtime.rs` 封装 `window.location.origin`、`window.localStorage` 持久化与同源 `XMLHttpRequest` 传输，统一 `net.rs` 桌面/Web 全量云端接口；Web 端启动直接进入不可关闭的全屏云端登录页，已登录时自动恢复并支持云端使用统计与登出回登录页，并经 `app_flows` 与本地 dev API 实机浏览器端到端验收。
- **2026-11-24**：完成发版前全链路加固——① 接入 `/ws/workspaces/{ws}/documents/{doc}?v=1` 实时 CRDT 同步（`ws_transport.rs` + `doc_sync_session.rs` + `crates/mica-sync`）与 `/ws/workspaces/{ws}/views` 侧栏热刷新，云端编辑期禁用 HTTP `replace_all` 整篇覆盖；② 桌面端 `VaultStore::open_default()` 无缝兼容旧版 Flutter `store.db` 路径并仅迁移非敏感 UI 偏好；③ Web 端彻底切回 `HttpOnly` Cookie + `POST /api/auth/refresh` / `POST /api/auth/logout`（`localStorage` 零 Token 落盘）；④ 修正 `deploy/Dockerfile.web` 产物校验（`mica_rust_bg.wasm` + `mica_rust.js` + `nsh.js` 及 `.br`）并通过 `mica-web:rehearsal` 镜像构建与全栈实机浏览器预演。
- **2026-11-26（并发 401 刷新吊销与静默掉登假空白页修复）**：修复桌面/Web 端并发 401 同时用旧 `refresh_token` 刷爆服务端单次轮换重放检测导致整族吊销的问题（`cloud_sync::RefreshGate` 单飞刷新协调器 + 60s 轮换缓存 + RAII 取消守卫），补齐 `DocSyncController` / `ViewsWsWatcher` WebSocket 401 自动单飞刷新重连，新增主界面顶部「云端登录已过期」横幅（`cloud-relogin-banner`）与未缓存/拉取失败页面的遮罩卡（`doc-load-error-overlay`），彻底杜绝云端页面在掉登或失败时伪装成空白页（回归：`cloud_sync::tests::concurrent_401_burst_issues_single_refresh_without_triggering_reuse_revocation`）。
