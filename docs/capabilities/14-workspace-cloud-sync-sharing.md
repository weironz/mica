# 14 — 工作区管理、本地/云端世界切换、同步与公开分享

> **适用范围**：桌面端、Web、移动端。

## 核心不变量

1. **本地与云端按源（Origin）严格隔离**：本地离线 SQLite Vault 与多个不同云端服务器（`origin`）的数据、凭据、工作区与同步队列按 `origin` 完全隔离，切换世界绝不串数据或泄漏凭据。
2. **异步网络响应必验归属**：登录、拉取树、同步文档或获取成员列表跨 `await` 返回时，必须核对当前活动 `origin`/账号/会话代际；切换世界后的在途旧响应一律丢弃。
3. **桌面端无账号可离线使用，Web 端纯云端无本地工作区**：桌面端支持零注册、零联网的本地离线 SQLite Vault（默认无缝兼容旧版 Flutter `store.db` 路径并保留早期纯 Rust `vault.sqlite` 回退与 `MICA_DATA_DIR` 覆盖）；Web 端没有持久本地 SQLite 库，严禁启动或回退到假「本地工作区」，直接绑定当前 `window.location.origin` 作为纯云端客户端，未登录时展示不可关闭的全屏云端登录门禁；Web 鉴权走服务端 `HttpOnly` Cookie（`mica_token` + `mica_refresh`）与 `POST /api/auth/refresh` / `POST /api/auth/logout`，`window.localStorage` 仅保存非敏感 UI 偏好、严禁落盘任何 Token。
4. **云端文档实时 CRDT 协同优先，禁止并发整篇覆盖**：已打开的云端文档默认建立 `/ws/workspaces/{ws}/documents/{doc}?v=1` WebSocket 连接并由 `crates/mica-sync` `SyncEngine` 驱动增量 yrs diff 同步（同时订阅 `/ws/workspaces/{ws}/views` 热刷新目录树）；WS 协同激活期间跳过 HTTP `PATCH /markdown`（`replace_all`），避免并发编辑互相覆盖。
5. **单飞令牌轮换、长期滑动会话与绝不展示假空白页**：Refresh Token 默认有效期 **365 天（滑动续期）**，活跃使用持续顺延；客户端所有云端 HTTP 401 刷新与 WebSocket 401 重连刷新必须经由按 `(server_url, refresh_token)` 归并的单飞（single-flight）协调器（含 RAII 取消唤醒与短时轮换结果复用缓存；Web 端额外叠加 `navigator.locks` 跨标签页锁 + `localStorage` 非敏感刷新时间戳，避免多标签页并发重放旧 Cookie），防止并发请求触发服务端重放检测吊销整条登录族；一旦云端会话真正过期，主界面必须展示醒目的重新登录横幅，且未在本地缓存的云端页面必须展示「重新登录 / 重试」遮罩卡（严禁静默渲染成假空白文档）。

---

## 能力与交互规格

### 世界切换与多服务器账户
- **`CAP-14-01` 底部账号菜单、一键环境切换与堆叠卡片浮层**：侧栏底部常驻当前身份行（桌面端本地模式显示「本 / 本地工作区 / 这台设备」；云端显示用户头像与邮箱），点击弹出堆叠卡片菜单（身份卡 + [桌面端]「切换环境」快速切换卡，最多 4 行高度 `140px` 限高内部滚动并将当前活跃云端置顶，列出本地离线库与已登录自托管服务器并带当前 `✓` 标记支持一键切换 + 设置/使用统计/[桌面端]「管理服务器…」/[桌面端]检查更新 + 退出登录或登录云端）。Web 端不展示「切换环境」卡、「管理服务器…」、任何本地模式文案与「检查更新」入口，点击「退出登录」清除当前 `origin` 凭据并立即回到不可关闭的居中单卡片云端登录页。
- **`CAP-14-02` 桌面端居中环境切换弹窗与 Web 居中单卡片登录页**：
  - **桌面端（`allow_local == true`）**：取消占屏 70% 的全屏深色海报「登录首页」，改为盖在工作区之上的 `440px` 居中模态弹窗，采用扁平环境列表 + 单服务器认证表单两视图设计（参考原型 [world-switcher-demo.html](../features/world-switcher-demo.html)）：
    - **View A（`world-env-list`）**：同一份列表平铺「本地离线库」与全部已配置自托管服务器（显示域名、状态点、已登录邮箱或未登录/需重登徽标），已登录主机点击直接免密切入、未登录主机点击滑入 View B 并自动探测 `/api/health`，支持行内移除确认与底部 `+ 添加自托管服务器` 内联输入。
    - **View B（`world-auth-form`）**：顶部 `←` 返回环境列表 + `登录到 {host}` 标题 + 紧凑服务器连通状态条（带手动重测按钮）+ 去双层分段标签的干净邮箱/密码表单（底部文字链接切换「忘记密码？」与「没有账号？注册 / 已有账号？去登录」）；从顶部「重新登录」横幅、加载失败遮罩卡或菜单「登录云端…」进入时直接落到 View B。
  - **Web 端（`allow_local == false`）**：取消深色营销左栏与多服务器下拉，在暖灰纸张底色上展示 `396px` 居中单卡片登录页（Mica 图标 + 标题 + 当前同源服务器健康状态条 + 干净登录/注册/忘记密码表单 + 365 天滑动会话说明），未登录时隐藏关闭按钮。

### 工作区管理、跨工作区迁移与成员邀请
- **`CAP-14-03` 工作区下拉切换菜单**：`320px` 宽工作区菜单（超过 8 个工作区自动出现搜索过滤框）、显示页面计数、每行 `⋯` 菜单支持重命名、导出（Mica 备份 / Markdown ZIP）、导入（文件/文件夹）、删除；底部提供「+ 新建工作区」与默认折叠的「导入工作区」（点击展开两行去括号紧凑选项：`从 ZIP 导入` + 右侧浅灰 `Mica / Markdown`，以及 `从 Notion 导入` + 右侧浅灰 `Markdown & CSV`）。
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
| `CAP-14-05` 工作区设置与成员邀请 | P1 | 桌面/Web | 🟡 | UI 与 `net::tests` 本地 mock 已通；`e2e/web_e2e.mjs` `[10/10]` 已补工作区设置弹窗黑盒检查（待下次 CI 实跑转绿） |
| `CAP-14-06` / `07` 公开分享与 API 令牌 | P1 | 桌面/Web | 🟡 | `share_dialog::tests` + `net::tests` 已通；`e2e/web_e2e.mjs` `[10/10]` 已补 `/s/{token}` 开关→200→关闭→404 黑盒测试（待下次 CI 实跑转绿） |

---

## 演进记录

- **2026-11-23**：沉淀本地/云端按源隔离、世界切换、工作区治理与公开分享规格。
- **2026-11-23**：在纯 Rust 客户端补齐云端登录后的工作区列表拉取（`GET /api/workspaces`）、按工作区隔离的视图树镜像（`GET /api/workspaces/{ws}/views`，兼容服务端 `object_type="document"`）、文档 Markdown 读写与 401 自动 refresh token 轮换，以及启动自动恢复与登出回切本地。
- **2026-11-24**：移除 Web 端（`wasm32`）的本地工作区与本地模式入口；新增 `web_runtime.rs` 封装 `window.location.origin`、`window.localStorage` 持久化与同源 `XMLHttpRequest` 传输，统一 `net.rs` 桌面/Web 全量云端接口；Web 端启动直接进入不可关闭的全屏云端登录页，已登录时自动恢复并支持云端使用统计与登出回登录页，并经 `app_flows` 与本地 dev API 实机浏览器端到端验收。
- **2026-11-24**：完成发版前全链路加固——① 接入 `/ws/workspaces/{ws}/documents/{doc}?v=1` 实时 CRDT 同步（`ws_transport.rs` + `doc_sync_session.rs` + `crates/mica-sync`）与 `/ws/workspaces/{ws}/views` 侧栏热刷新，云端编辑期禁用 HTTP `replace_all` 整篇覆盖；② 桌面端 `VaultStore::open_default()` 无缝兼容旧版 Flutter `store.db` 路径并仅迁移非敏感 UI 偏好；③ Web 端彻底切回 `HttpOnly` Cookie + `POST /api/auth/refresh` / `POST /api/auth/logout`（`localStorage` 零 Token 落盘）；④ 修正 `deploy/Dockerfile.web` 产物校验（`mica_rust_bg.wasm` + `mica_rust.js` + `nsh.js` 及 `.br`）并通过 `mica-web:rehearsal` 镜像构建与全栈实机浏览器预演。
- **2026-11-26（并发 401 刷新吊销与静默掉登假空白页修复）**：修复桌面/Web 端并发 401 同时用旧 `refresh_token` 刷爆服务端单次轮换重放检测导致整族吊销的问题（`cloud_sync::RefreshGate` 单飞刷新协调器 + 60s 轮换缓存 + RAII 取消守卫），补齐 `DocSyncController` / `ViewsWsWatcher` WebSocket 401 自动单飞刷新重连，新增主界面顶部「云端登录已过期」横幅（`cloud-relogin-banner`）与未缓存/拉取失败页面的遮罩卡（`doc-load-error-overlay`），彻底杜绝云端页面在掉登或失败时伪装成空白页（回归：`cloud_sync::tests::concurrent_401_burst_issues_single_refresh_without_triggering_reuse_revocation`）。
- **2026-11-26（多主机环境切换与登录 UI 重构）**：彻底移除全屏 70% 深色海报与双层嵌套分段标签。桌面端改用居中 `440px` 弹窗（扁平环境列表 View A「本地离线库 + 自托管服务器 + 添加/移除」↔ 单服务器认证表单 View B）并在左下角账号菜单新增「切换环境」一键快速切换卡；Web 端改用 `396px` 暖灰底居中单卡片登录页；补齐点击「重新登录 / 登录云端…」直达 View B 与 `app_flows` 回归验证。
- **2026-11-26（跨工作区移动图片加载失败修复）**：修复三大根因——① `app_transfer_io.rs` 在云端模式下此前仅在本地 SQLite 伪克隆且未调用服务端 `POST /api/workspaces/{ws}/views/batch-transfer`（导致目标工作区缺少物理复制的 blob 与重写的 `file_id`，并补齐云端目标目录树拉取与真 dry-run 预览）；② `editor_bridge::export_markdown` 与后台防抖落盘此前传空图片映射表导致 `file_id` 退化成裸文件名 `pasted-image.png`，接入 `build_blocks_image_urls` 后云端保留可被 `rewire_blob_hrefs` 识别的永久 blob URL、本地保留 Desktop CAS `file:///` URI，且 `resolve_image_block` 识别带完整 origin 的自有 blob 链接不再误标为外链；③ 服务端 `transfer_roots` 补充识别未携带 `file_id` 仅留源工作区 blob `url`/`src` 的图片块并统一复制与重写。
- **2026-11-27（公开分享开关无法开启修复）**：`share_dialog_modal` 的 `公开访问` 开关位于 `dynamic(...)` 体内，点击触发 `share_busy.set(true)` 后直接裸调 `spawn(...)` 会将异步任务绑定在即将被重建销毁的临时子 `Owner` 上，导致 `POST/DELETE /share` 在发出瞬间即被取消且 `share_busy` 永久卡在 `true`（置灰不可点）。修复为统一在 `app_owner.run` 下派发分享加载/开关及其他弹窗异步任务、接入 `with_auto_refresh` 自动续期与错误内联展示，并在 `share_dialog.rs` 中将复制链接的复原定时器绑定至 `modal_owner`。
- **2026-11-27（Web 首屏秒开壳、WASM LTO 瘦身与哈希资源 `immutable` 缓存）**：① [web.rs](clients/mica_rust/src/web.rs) 将 `rust_app!` 的 `ui_sets` 从 `vec![nsh_base::registry()]` 改为 `Vec::new()`，并将 Mermaid 源码编辑框改为原生多行 `input()`、移除 `nsh-base` 的 `editor` 特性与新增 `[profile.web] strip = true`，解除全部 75 个未使用 `nsh/base` TSX 注册组件（`syntect`/`html5ever`/`ratex`/`vte`/`zstd`/`plot`/`rich_editor` 等）对 LTO 死代码裁剪的强引用；② [index.html](clients/mica_rust/web/index.html) 改为 T=0 并行启动 `fetch(wasm)`（流式进度 + `compileStreaming`）、`import(nsh.js)` 与单飞 `/api/auth/refresh` 探测，未登录（401/无痕）1 RTT 内直出 1:1 HTML/CSS 登录/注册/找回密码页，已登录直出工作区骨架屏，认证成功后通过内存临时 `window.__micaPreAuth` 零 `localStorage` 秘密交接给 Rust 跳过重复刷新；③ [nginx.conf](deploy/nginx.conf) 对带 `?v=<sha256>` 哈希的 `.js`/`.wasm` 启用 `public, max-age=31536000, immutable` 强缓存（入口 `index.html` 保持 `no-cache`）并同步双份 nginx CSP sha256 哈希。
- **2026-11-28（`e2e/web_e2e.mjs` 补齐公开分享/工作区设置黑盒测试）**：在 `[1/10]` 补登录页 `#ml-main` SVG 渐变自含校验，新增 `[10/10]` 覆盖云端公开分享开启→未鉴权访问 `/s/{token}` 200→关闭→404 撤销与工作区设置弹窗验证。
- **2026-11-28（工作区切换菜单底部导入区折叠与去括号排版优化）**：将工作区下拉菜单底部的常驻 3 行导入区改为默认折叠的「导入工作区」同级行（`ws-import-toggle` + 图标 + 右侧箭头），点击就地展开两行单行紧凑子项（`ws-import-zip` / `ws-import-notion`），去掉主标题中的冗长括号说明，将 `Mica / Markdown` 与 `Markdown & CSV` 作为右侧浅灰格式标签展示，并补齐 `app_flows` 断言。
- **2026-11-28（账号菜单「切换环境」限高滚动与文案精简）**：为「切换环境」快速切换列表接入 `140px`（约 4 项）`max_h` 内部滚动容器并将当前活跃云端置顶，防止多服务器撑高甚至溢出窗口；将底部重复入口「切换服务器 / 本地库…」精简为「管理服务器…」，并补齐单元测试与 `app_flows` 断言。
- **2026-11-28（Web 切换工作区、工作区浮层菜单与头像全量恢复）**：① 修复 Web Cookie-only 会话在 `require_auth` 误判未登录及同包缓存覆盖导致切换云端工作区空白，并严格以页面真实父文件夹链推导面包屑；② 将工作区切换器行内 `⋯` 动作菜单迁移至顶层 `overlay()` 浮层（Web 隐藏本地文件夹导入），不再挤压遮挡下方工作区列表；③ 接入账号头像字段、头像上传/移除与 `4` 处一致展示（左侧栏底栏、账号菜单卡片、设置账户页、工作区成员列表），并将设置页资料/改密/注销、PAT 令牌与云端 AI 配置绑定真实云端 API。
