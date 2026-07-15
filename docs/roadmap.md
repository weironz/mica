# Mica 路线图 — 剩余功能与优化点

> 2026-07-08 生成。来源:多代理系统扫过 `crates/` + `clients/mica_flutter/lib/` 的
> TODO/未做标记、`docs/` 的 pending 项、编辑器里程碑、后端硬化面,综合排优先级。
> 影响力从高到低;`(S/M/L)` = 工作量;`[需后端]` = 要动 Rust。
>
> 背景:v0.1.4。**M-R 云端数据安全里程碑已完成**(崩溃/切页/坏 update/流截断四类
> 丢数据 + 熔断可见,见 `phase2-offline-crdt.md` §13),自动重连见 §13.1。

## 可靠性与同步

- **P2-M4 云同步流未真正建**(bigserial 单调流 + 断点续传 + SV 回退 + local-seq→Rid)—— 离线优先同步的主干,现有 op 模型本应随它退役。(L) `[需后端]`
- **实时字符级并发协同未落地** —— presence 光标已画(`render.dart`),但同块并发输入仍靠 last-write,「协同」名不副实。(L) `[需后端]`
- **M-R 收尾 C3/D1/D2/A3** —— 坏更新加载自愈 + schema 版本号、静默 `catch{}`→计数日志、同步健康态、会话持久化 e2e。(M)
- **离线→在线 blob 自动 reconcile** —— 现只在重开文档时懒重传;可挂到自动重连成功事件上。(M) `[需后端]`
- **双向 state-vector 协商** —— bootstrap 永远发整档 base(`ws.rs`),server 存了 SV 却不算 diff,大档新客户端很贵。(L) `[需后端]`
- **broadcast lag 触发整档重载** —— 已有 rid cursor + `sync.pull`,lag 本可增量续拉而非重载(`ws.rs`)。(M) `[需后端]`
- ~~客户端自动重连~~ ✅ 已做(branch `feat/cloud-auto-reconnect`,退避重连,§13.1)。

## 安全

> 上一轮安全 review 的落地清单。自托管一上公网,前几项是硬底线。

- **无 refresh / 无撤销的 24h JWT** —— 令牌被盗 24h 内无法吊销,`JWT_SECRET` 一改全员掉线(`auth.rs`, `config.rs`)。(L) `[需后端]`
- **改密不失效旧令牌** —— 泄露后改密码登不掉攻击者(`auth.rs`)。(M) `[需后端]`
- **登录/注册/WS 无限流** —— 每请求跑一次 Argon2,可在线爆破 + CPU DoS(`auth.rs`)。(M) `[需后端]`
- **自托管 TLS 全靠运维 + `HTTP_ADDR` 默认明文** —— 叠加 query token,未配 TLS 即明文泄露,且无启动告警(`config.rs`)。(M) `[需后端]`
- **鉴权逐 handler 手写、非中间件** —— 新路由默认不鉴权,忘加即漏(`main.rs`, `auth.rs`)。(M) `[需后端]`
- **WS token 走 query string** —— 明文 JWT 落反代日志/浏览器历史(`ws.rs`)。(M) `[需后端]`
- **长连 WS 超 token TTL 不再认证** —— 过期前建的 socket 可授权数小时,无 re-auth 心跳(`ws.rs`)。(M) `[需后端]`
- **CORS 全放行**(`CorsLayer::permissive()`)—— 应收紧到配置 origin(`main.rs`)。(S) `[需后端]`
- **桌面 token 明文存 prefs**(无 DPAPI)(`main.dart`)。(M)
- **开放注册无验证 + 弱口令(仅 ≥8)** —— 公网可无限刷号(`auth.rs`)。(M) `[需后端]`

## 编辑器与功能广度

- **全文搜索是无索引 O(N) 子串扫描** —— 每查询反序列化每篇快照做 `contains`,无分词/排序/高亮,随空间线性劣化(`documents.rs`)。(L) `[需后端]`
- **表格未完成** —— 单元格是纯 `List<List<String>>`,无富文本 marks/矩形区选/合并(`table.dart`,editor-engine M6)。(M)
- **无反向链接/引用面板/关系图** —— 正向 `[[` 已建,缺反向索引(wiki 类的定义能力)。(L) `[需后端]`
- **无标签/页面属性/数据库视图** —— 对象模型只认 `document`(`documents.rs`)。(L) `[需后端]`
- **评论/建议未建** —— 仅 `commenter` 角色打通,marks 模型本为 range 锚点预留。(L) `[需后端]`
- **callout/toggle/embed/columns 块未建** —— Notion 类常见结构块。(L)
- **无屏幕阅读器语义(a11y) / 无 RTL 双向文本** —— 自绘 RenderBox 无 Semantics;10+ 处硬编码 `TextDirection.ltr`(editor-engine, `render.dart`)。(各 L)
- **文档内查找/替换缺失** —— 有全局搜索却无 Ctrl+F;基于现有文本模型很便宜。(S)
- ~~**行内数学未排版**~~ ✅ 2026-07-16:`$…$` 真排进行里(基线对齐、随字号缩放),公式为不可进入的原子 —— 点击弹源码编辑框、光标两侧跳过、退格删整体(AppFlowy/Notion 交互)。机制 = `InlineAtomRenderer` 注册表 + `FoldPlan` doc↔painter 映射 + `setSelection` snap(`inline_atoms.dart`,render-architecture.md Decision 4)。
- **Web IME/光标滚动实况调优** —— Milestone 1 遗留(合成态/游离换行、caret scroll-into-view)。(M)
- **AI 离线为空 stub / 无拼写检查 / 无字数统计**。(M / M / S)

## 平台覆盖

- **无触屏选择手势** —— 无长按选词/选择手柄/放大镜,手机端文本选择基本不可用。(L)
- **Windows 未签名(SmartScreen 告警)** —— 路径:SignPath CA 证书接入 Inno SignTool(desktop-plan)。(M)
- **无内置自动更新** —— 现靠手动;可采 AppFlowy 的 WinSparkle + appcast(desktop-plan)。(L)
- **window_manager→nativeapi / Turso 观望**(各 S,已隔离在 trait 后)。

## 性能

- **长文档无虚拟化** —— 单 `RenderDocument` 每帧布局+绘制全部节点,大档每击键全量重排(editor-engine)。(L)
- **每次 push 重建+重编码+重写整档(写放大)** —— `from_update`→全档 `encode_state`+upsert,成本 O(文档) 而非 O(更新)(`sync.rs`)。(M) `[需后端]`
- **yrs base 无 squash/GC,无界增长** —— 只裁 stream 不压 base,长寿文档 base 越滚越大(`sync.rs`)。(L) `[需后端]`
- **本地持久化仅全量快照** —— §4 的增量队列 + squash 折叠推迟中。(M)
- **frb v2 热路径 FFI 基准待测** —— IME/逐字输入若过慢,热路径留 Dart(phase2 §12)。(M)

## 开发者体验 / CI / Markdown

- ~~CI 不跑测试~~ 🟡 部分已做(branch `ci/add-tests`:Rust 纯 crate + Flutter 单测;**待补** Postgres 依赖测试 + Windows 集成测试入 CI)。(M) `[需后端]`
- **仅结构化日志,无 /metrics/telemetry** —— 同步后端生产盲飞(`telemetry.rs`)。(M) `[需后端]`
- **可选/later 基建:Redis、OTel、索引块表** —— 索引块表是搜索/反链/分析的底座,值得与搜索一起规划(architecture.md)。(L) `[需后端]`
- **自研 parser vs 采用 comrak(读侧)未决** —— Milestone 8 决策点(editor-engine)。(M)
- **catch-up limit / stream 常量硬编码** —— 1000、KEEP_MARGIN/PRUNE_EVERY 应入 AppConfig(`ws.rs`)。(S) `[需后端]`
- **过时注释/文档批量清理** —— 多处 "M5+/later" 已实现却没更新(`main.dart`, `model.dart`, `lib.rs`, `preview_raster.dart`)。(S)

## 接下来最该做的 3–5 件

1. **CI 接 `cargo test` / `flutter test` + sync 集成测试入库**(high / M)—— 刚做完数据安全里程碑,核心 CRDT 数据面却无回归网,先锁住再往前。〔已起头:branch `ci/add-tests`;待补 Postgres 依赖测试 + Windows 集成测试。〕
2. **客户端自动重连 + 网络监听**(high / M)—— ✅ 已做(branch `feat/cloud-auto-reconnect`);后续把 blob pending 重传 + 细粒度状态 UI 挂到重连事件。
3. **限流 + 收紧 CORS**(high / S–M)—— Argon2 逐请求跑,当前可在线爆破 + CPU DoS;公网自托管的硬底线,成本低。`[需后端]`
4. **Token 撤销(per-user token-version 表)**(high / M)—— 一张表同时补上「改密失效」+「强制下线」两个缺口。`[需后端]`
5. **文档内查找/替换**(medium / S)—— 基线编辑器能力、高频、基于现有文本模型即可落地,性价比最高的功能补齐。

---

**整体判断**:安全 + CI 是「发出去前必须补的底线」;自动重连是离线优先的功能完整性(已补);
再往后**虚拟化 + 表格 + 反链**决定它像不像一个成熟笔记。
