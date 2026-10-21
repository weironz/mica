# 第四轮:评审修复与结构收口(2026-10-21)

依据:[`../code-review-2026-10-21.md`](../code-review-2026-10-21.md)。条目编号沿用报告里的(P1-x / S / D / C / U)。

**执行规矩(CLAUDE.md「修复纪律」):**
- 每一条都要带回归测试，并做实测验证。web 用 playwright 截图，桌面相关的写明「待真机」。
- 提交信息写根因。
- 先确认失败、再修。尤其是标【需实测】的性能条目：先写基准、确认能复现，再动代码。避免出现上一个触控板测试那种「不带修复也通过」的真空测试(lessons §3)。
- 每个阶段结束跑一遍 `just ci`:clippy、Rust+DB 测试、flutter analyze/test、结构门禁。

## 阶段 0 — 依赖与一行级止血(一个补丁版)

改动小、风险低、收益直接，合起来发一版。

| 项 | 动作 | 回归测试 |
| --- | --- | --- |
| 依赖漏洞 | `cargo update -p h2 -p quinn-proto -p rustls -p rustls-webpki -p webpki-roots`(都在兼容范围内);重跑 `cargo audit` 确认归零;rmcp 不受影响，不升 major | `cargo audit` 结果写进提交信息 |
| 未用依赖 | 删除 app-core `thiserror`、mcp-server `tracing`、infra `serde_json` | 编译通过 |
| P1-6 深度上限 | `quote_depth_of` / `li_of` 加 `.min(8)`,常量和 Dart `model.dart` 共用同一个值 | 渲染 5 万层 `>`、写入 `data.li=1e9` 后导出，都不崩 |
| C10 ZIP64 | 读之前先 `end.min(extra.len())` | 构造坏 extra 的 zip,导入返回 400 而不是 panic |
| S1 头像 SVG | 头像只接受位图;Content-Type 改用 `stored_content_type` | 上传 SVG 头像返回 400 |
| P1-2 切页签 flush | `_selectTab` 先 flush;编辑器 dispose 时同步 flush,并取消 `_retryTimer` | widget 测试：打字后立即切页签，断言 op 已发出、并且发到原文档 |
| U4 更新器 | `exit(0)` 之前先跑 `appExitFlush` | 单测:mock 掉 exit,断言 flush 先被调用 |
| `kDevAutoLogin` | 条件改为 `&& !kReleaseMode` | — |
| `just check` | 去掉 `2>/dev/null \|\|`,clippy 失败就失败 | — |

## 阶段 1 — 数据正确性

| 项 | 动作 | 备注 |
| --- | --- | --- |
| P1-1 MCP 路径穿越 | args 里的 id 改成 `Uuid`;URL 改用 `path_segments_mut` 拼接;补上 mcp-server 的第一批「工具实际发出什么请求」测试 | 同时修 C11:所有改动文档的工具都清缓存 |
| P1-5 标题 round-trip | 导入时文档也读 manifest 的 `title`;标题的写入(`with_page_title`)和剥离做成 `mica_markdown` 里的一对函数，剥离时跳过 front matter | 闭环测试用「Meeting notes」、带 front matter、同名加 `-2` 后缀三种名字 |
| P1-3 撤销移动 | `_diffOps` 检测块顺序变化，补发 `move_block` | 把 `move_block_test` 改成断言发出的 op |
| C8 redo | redo 前先 flush,和 undo 对称 | — |
| 本地库一组(C3–C7) | 这几条在同一个文件里，一起修:<br>· `delete_doc` 也删 `doc_version`<br>· 搜索/反链/图谱加 `origin='local' AND workspace_id=?`<br>· `clone_view` 复用 `subtree_ids`,并过滤掉回收站里的页面<br>· `appendUpdate` 返回 0 时用 `saveDoc` 全量自愈<br>· restore/rollback 改为先解码，再在同一个事务里写库 | mica-core `--features store` 测试 + FFI 测试 |
| C2 父节点 TOCTOU | 触发器 `views_parent_must_be_folder` 扩成同时要求父节点 `is_deleted=false`,且 `workspace_id` 一致 | **新迁移**,绝不改已发布的迁移(lessons §12)。按 §13 判断：发布时是单向门 |
| C1 transfer 竞态 | 事务内重新枚举子树，和第一步的集合比对;不一致就返回 409 并回滚 | pg 测试 |
| U1/U2/U3 外壳竞态 | · `_selectView` 和 `_restoreSession` 进入时记下 tab 和 origin,每个 await 之后核对一次<br>· 后台页签加载失败改走 `_closeTab` | widget 测试 |
| P1-8 purge_view | **需用户拍板**(roadmap 记为独立决定)。推荐：单条改为调用 `purge_views_batch(&[id])`,和批量版行为一致 | 拍板前不动 |
| 预热/回填竞态 | 回填改正文时一并更新 `updated_at`;派生列 upsert 改用 `clock_timestamp()` | — |

## 阶段 2 — 安全加固

| 项 | 动作 |
| --- | --- |
| S2 凭据纪元 | 一组改动:<br>· JWT 加 `iat`,users 表加 `credential_epoch` 字段<br>· 铸 PAT 必须用 JWT 发起，并校验纪元<br>· `resolve_pat` 关联 users,排除已删账号(墓碑) |
| S3 AI key | `list_models` 改成 POST body,客户端同步改 |
| S4 CSP | 分享页和导出文件共用一个 CSP 常量，加上 `form-action 'none'; base-uri 'none'`;tagfilter 丢弃 `meta http-equiv` |
| S5 rehost SSRF | 禁用重定向;拒绝私网/环回地址;要求 `image/*`;加大小上限。和服务端 `files.rs` 的防护逻辑同源 |
| S6 metrics | 非标准的 HTTP method 统一归为 `OTHER` |
| 旧条目 | · XFF 只在对端属于可信代理段时才采信<br>· `ai/settings` 只回给管理员<br>· `APP_ENV` 改为 fail-closed:只认 `development`,其他值一律按 production<br>· `telemetry.rs` 默认日志级别改成 info |
| D1/D2/D3 | · Argon2 走 `spawn_blocking`,permits 设为核数/2<br>· WS 消息上限 4 MiB<br>· AI 预览限制输入大小，并放进 `spawn_blocking` |
| 超时统一 | · mcp 和 Flutter 的 HTTP 客户端加 connect + read 超时<br>· cli 的上传/下载单独设超时<br>· api-server 用一个共享的、带超时的 reqwest client |

## 阶段 3 — 性能(先基准后改)

| 项 | 动作 |
| --- | --- |
| P1-7 内联嵌套 | 先在 `nested_bracket_perf.rs` 加交替嵌套的用例，确认确实是指数级;然后让缓存贯穿整次解析，并加深度预算 32。**Rust 和 Dart 两端一起改** |
| D4 平方级 | 预扫描一遍，记下每个位置「下一处 `\)`、下一处边界」;html_span 改成按起点排序后线性推进 |
| D5 缓存循环 | 预览缓存和图片缓存改为 LRU(在 paint 时 touch),只对视口附近的节点发请求 |
| 编辑器 | · 光标在代码块里移动时复用布局，不再整段重新高亮<br>· `setHover` 合成一次扫描 |
| 服务端解码 | 合成一个 `write_base_tx`(见报告「精简」一节);`block_index` 改用 id→下标映射 |

## 阶段 4 — 精简(纯重构，不改行为)

按报告「精简、死代码与重复」一节执行，预计约 −2,200 行。顺序：先删死代码(风险最低),再做「单条复用批量」,最后合并重复实现。

- **单条复用批量**:delete/restore/purge 三对操作收成一个入口。这是「兄弟路径漏守卫」的结构性解法，不再一处一处地补。
- **跨 crate 共用**:`unique_asset_name` / `safe_segment` / `percent_decode` 统一放进 interchange;mica-core 直接 `pub use` markdown 的 `Block`。
- **Dart**:
  - 图片 MIME 嗅探、SHA-256、前缀 diff 各保留一份。
  - 更新器改用自研的 sha256,删掉 `crypto` 依赖。
- **注释**:错挂、过时的注释一次清理干净。
- **`legacy_orphans.rs`**:等宽限期结束再删，并在 release.md 里记一笔。
- **验收**:每一步行为不变，现有测试全绿;`scripts/file-size-budget.txt` 的基线随之收紧。

## 阶段 5 — 架构

| 项 | 动作 | 为什么排在最后 |
| --- | --- | --- |
| P1-4 协作 Undo | 短期：有协作者在场时，把全域 undo 降级为只撤销本地最近一段输入。长期：改为记录本地 op 的逆操作(Yjs UndoManager 的 tracked-origins 思路)。先调研 AppFlowy / AFFiNE 的做法(原则 6) | 结构性改动，需要单独立方案 |
| 路由分层 | 公开路由 / 限流改为路由级 layer,取代三张路径后缀表 | 牵动整个鉴权面，需要完整回归 |
| 外壳 `WorldBackend` | 本地/云端统一走一个接口分派，消掉 81 处 `local ?` 三元判断;dialogs 的设置页按页签拆开;presence 并进 `CloudSyncSession` | 文件大、耦合深，放在前几阶段止血之后 |
| 渲染注册表 | `AtomicBlockRenderer` 加 `hitTest` / `paintSelection`,把表格的 9 个 hit-test 方法和 setHover 分支收进去 | 守红线，不急 |
| 契约测试 | 服务端为关键 DTO 产出 gold JSON,cli / mcp / Dart 三方各自解码断言 | 比建共享 crate 成本低 |
| 工程链 | · 结构门禁改为按库计行数，并纳入 FFI crate<br>· 执行 `cargo fmt --check`<br>· workflow 抽 composite action,统一 action 版本<br>· `just test` 复用 lib-local 的数据库探测 | — |
| 文档 | 按代码重写 `architecture.md`;补齐依赖豁免清单 | — |

## 明确不做(以及为什么)

- **不建 cli / mcp / api 共享的 DTO crate**:web 端必须有一份 Dart 实现，共享 crate 只能覆盖其中两方。用契约测试守住漂移，成本更低。
- **不把 api-server 的 SQL 大规模搬进 app-core**:app-core 实际是「文档/同步域 + AppState」。改 `architecture.md` 让它和代码一致，比为了符合文档去搬代码划算。
- **不升 rmcp 的 major 版本**:漏洞只在我们没用的 HTTP 服务端传输里。
- **`ws_min_protocol` 暂不提高到 1**:会断开还没升级的老客户端，需要用户拍板，并配合一次发版公告。

## 需要用户拍板的

1. `purge_view` 是否收紧为只能删回收站里的页面(推荐收紧)。
2. `add_member` 目前不需要对方同意，任何成员都能把工作区加进别人的切换器里;是否改成邀请制。
3. `legacy_orphans.rs` 的删除时机。
4. 协作 Undo 短期降级方案的产品表现。

## 实施记录

(按阶段追加：日期、提交、验证方式。)
