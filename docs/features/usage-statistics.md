# 使用统计与可观测性设计方案（Issue #191）

> 状态：**设计评审定稿（待实施）** · 对应需求：[#191](https://github.com/weironz/mica/issues/191)
> 更新日期：2026-11-20

## 1. 背景与目标

随着知识库体量增长（以当前生产实例为例：30 个工作区、13,667 篇活跃页、2,720 个文件夹、4,960 万字符、5,537 个图片与附件），用户和运维同时需要三类信息：

1. **资产盘点**：账号/本机一共有多少工作区、页面、字数、图片、附件以及存储容量分布。
2. **创作与维护节奏**：按今天、近 7 天、近 30 天、近 90 天、近 1 年观察新建页数、编辑活跃度、写作字数变化以及连续活跃热力图。
3. **运维可观测性**：通过现有 Prometheus `/metrics` 端点持续采集存量与增量指标，支撑容量增长预警与 Grafana 看板，并提供独立的指标参考文档 `docs/ops/metrics.md`。

### 明确的非目标（刻意不做）

- **不做页面阅读量 / PV 埋点**：个人与小团队知识库引入读埋点会显著放大写库负载并带来隐私负担，性价比远低于编辑活跃度。
- **不做编辑停留时长计时**：窗口常驻挂机极易失真，无法真实反映投入。
- **不做秒级实时推送**：统计视图在打开或切换筛选时按需拉取，`/metrics` 继续复用现有 60 秒 DB 快照缓存。
- **不在 Prometheus 中暴露 `user_id × window` 维度**：控制指标基数并守住运维接口与个人隐私边界。

---

## 2. 核心决策与口径规范

### 2.1 入口与展示形态

- 复用现有宽幅设置弹窗 **`MicaSettingsShell`**（最大 `1120 × 840`，`< 700px` 自动切换为列表→详情两步布局）。
- 在设置侧栏增加独立导航项 **「使用统计」**（`MicaSettingsDestination`），归入「工作区 / 数据」分组，云端模式与桌面本地离线模式均可见。
- 在左下角 **`AccountMenu`** 增加 **「使用统计」** 快捷入口；点击后直接打开 `SettingsDialog(initialSection: SettingsSection.usageStats)` 定位到该页签，避免再维护一套独立弹窗壳。

### 2.2 云端与本地离线同构（Rust-first 单一真相源）

遵循项目 Rust-first 数据面不变量：

- **字数统计单一实现**：将当前仅存在于 Dart 的中英文混排计数规则（[word_count.dart](../../clients/mica_flutter/lib/editor/word_count.dart)）在 `crates/mica-core/src/word_count.rs` 提供权威 Rust 实现，保证云端 Postgres、桌面本地 SQLite 与编辑器状态栏口径完全一致（CJK 汉字/假名/韩文每字计 1 词，连续拉丁/数字串计 1 词；`chars` 计全部非空白字符）。
- **统计模型与聚合复用**：云端（`crates/app-core` + `crates/api-server`）与本地离线（`crates/mica-core/src/store.rs` + 桌面 FFI）共享同一套指标定义、小时桶折叠函数和 DTO 结构。
- **前端面板 100% 复用**：新建独立 UI 库 `clients/mica_flutter/lib/ui/usage_stats_panel.dart`，只依赖统一的 `UsageStatsOverview` / `UsageStatsActivity` 数据模型与加载回调，不区分云端/本地写两套 UI（仅在本地模式将「配额进度条」显示为「无配额上限」）。

### 2.3 每日统计的「天边界」：业界实践与选型

**业界调研对比：**

| 方案 | 代表产品 | 优点 | 缺点 |
| --- | --- | --- | --- |
| **A. UTC 整点小时桶落盘 + 查询时按用户本地时区折叠** | GitHub Contribution Graph、Linear、现代 SaaS 分析 | 写入无需感知用户时区；跨时区出差/换设备不失真；表极小；凌晨 00:00–07:59 写作不会错算到「昨天」 | 查询时多一步按 `tz_offset` 归并整点（毫秒级，极简单） |
| **B. 写入时按固定服务端时区直接落 `DATE` 桶** | 早期后台报表 | SQL 最简 | 服务端跑 UTC 时，东八区早上 00:00–07:59 的写作会被算成昨天，体感割裂且历史不可逆 |
| **C. 存原始编辑事件明细表，查询时实时聚合** | 重型 BI 系统 | 任意粒度灵活 | CRDT 高频同步下写放大严重，表体积无界增长 |

**最终采用方案 A（UTC 小时桶 + 客户端时区偏移折叠）：**

1. **写入侧**：无论服务端 Postgres 还是本地 SQLite，统一将活动归入 **UTC 整点小时桶**（`bucket_hour`，即截断到整小时的 UTC 时间戳）。
2. **查询侧**：客户端在请求统计接口时自动附带本机当前时区偏移 `tz_offset_minutes`（如北京时间 `+480`，支持范围 `-720..=+840`），由服务端/本地 Rust 聚合层将每个 `bucket_hour + tz_offset_minutes` 折叠为本地自然日 `YYYY-MM-DD`。
3. **「今天 / 近 7 天 / 近 30 天」定义**：
   - `今天`：用户本地时区当日 `00:00` 至今；
   - `近 N 天`：包含今天在内的连续 `N` 个本地自然日（如 `7d` = 过去 6 天 + 今天），无活动的日期自动补 0，保证折线图与热力图连续无断档。

### 2.4 全量统计指标口径定义

| 指标 | 统计口径 | 备注 |
| --- | --- | --- |
| 工作区数 | 当前账号可见的云端工作区数（本地模式为本地工作区数） | 不含已删除工作区 |
| 活跃页面数 | `views` 中 `object_type = 'document' AND is_deleted = false` | 与侧栏可见页面数严格一致 |
| 文件夹数 | `views` 中 `object_type = 'folder' AND is_deleted = false` | 作为辅助维度展示，不混入页面数 |
| 回收站页数 | `views` 中 `object_type = 'document' AND is_deleted = true` | 辅助提醒清理空间 |
| 总字数 / 字符数 | 活跃页面对应的 `word_count` / `char_count` 之和 | 篇均字数 = 总字数 / 非空活跃页数 |
| 图片数 | `files` 中 `mime_type LIKE 'image/%'` 的记录数 | 本地模式按 CAS 扩展名/魔数区分 |
| 附件数 | `files` 中非 `image/*` 的记录数 | 与图片互斥，相加等于总文件数 |
| 已用容量与配额 | 复用 `store::workspace_bytes_used`（与上传配额拦截同源） | **单工作区**：`已用 / MICA_WORKSPACE_QUOTA_BYTES`（默认 5 GiB）+ 进度条；**账号总览**：`总已用容量` + `单工作区配额上限` + 最高占比工作区，**不把配额简单乘以工作区数**以免误导；**本地模式**：仅展示已用空间 |
| 知识健康度 | 双向链接总数（`link_targets` 边数）、孤岛页数（无入链且无出链的活跃页）、空页面数（`word_count = 0`）、公开分享页数（`document_shares`）、未解决评论线程数（`comment_threads`） | 全部由现有索引列/表直接聚合，零额外解码成本 |
| 时间维度：新建页数 | 窗口内新建的页面数（区分正常新建 vs 导入/转移来源） | 历史页面可由 `views.created_at` 回溯 |
| 时间维度：编辑页数 | 窗口内发生过正文编辑的**去重页面数**（按日图表按当日去重；汇总卡片按所选窗口去重） | 由小时-文档聚合表精确去重，不再被 `views.updated_at` 覆盖 |
| 时间维度：写作字数 | 窗口内每次保存正增量 `max(new_words - old_words, 0)` 累加（`words_added`），同时记录 `words_deleted` 与净增 `net_words` | 避免删除重构时「今日写了多少字」变成负数；界面主显新增字数，悬停提示净变化 |
| 连续活跃天数 | 从今天（或昨天若今天尚未写作）向前连续有新建或编辑活动的本地自然日数，以及历史最长连续天数 | 与热力图同源 |

> **历史数据边界说明（诚实不造假）**：
> `document_yrs_base` 只保存当前终态，旧版不留存每日字数增量。上线后，**存量总字数、页面数、知识健康度、历史新建页面分布**会通过启动回填立即完整呈现；**每日编辑页数与每日写作字数**自该功能上线之日起精确累积，UI 在时间范围早于记录起点时给出轻量提示。

---

## 3. 存储模型与写入路径设计

### 3.1 云端 Postgres（新迁移 `migrations/0040_usage_stats.sql`）

1. **`document_yrs_base` 增加衍生列**（与 `content_text`、`link_targets`、`file_ids` 遵循同一 co-write 与回填契约）：
   ```sql
   ALTER TABLE document_yrs_base
     ADD COLUMN word_count INTEGER,
     ADD COLUMN char_count INTEGER;
   ```
   - 保持 `NULL` 作为“待回填”哨兵；服务启动时的 `sync::backfill_projections` 分批补齐存量 1.5 万篇文档（预计数秒完成，不阻塞请求）。
   - 后续每次 `sync::write_base_tx`（涵盖 REST、WebSocket、MCP 全部写入路径）在已有 `blocks` 扁平化时顺手计算 `word_count` / `char_count` 一并落盘，零额外 CRDT 解码。

2. **新增小时-文档级轻量活动表 `writing_activity_doc_hourly`**：
   ```sql
   CREATE TABLE writing_activity_doc_hourly (
     user_id       UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
     workspace_id  UUID        NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
     document_id   UUID        NOT NULL,
     bucket_hour   TIMESTAMPTZ NOT NULL, -- date_trunc('hour', now() AT TIME ZONE 'UTC')
     source        TEXT        NOT NULL DEFAULT 'manual', -- 'manual' | 'import' | 'transfer'
     is_created    BOOLEAN     NOT NULL DEFAULT false,
     words_added   INTEGER     NOT NULL DEFAULT 0,
     words_deleted INTEGER     NOT NULL DEFAULT 0,
     edit_events   INTEGER     NOT NULL DEFAULT 0,
     PRIMARY KEY (user_id, workspace_id, document_id, bucket_hour)
   );

   CREATE INDEX writing_activity_doc_hourly_user_hour_idx
     ON writing_activity_doc_hourly (user_id, bucket_hour DESC);

   CREATE INDEX writing_activity_doc_hourly_ws_hour_idx
     ON writing_activity_doc_hourly (workspace_id, bucket_hour DESC);
   ```
   - **为什么带 `document_id` 而不是只按 `(user_id, workspace_id, bucket_hour)` 聚合？**
     - 体量依然极小：重度个人每天编辑 10 篇文档 × 2 个活跃小时 ≈ 20 行/天，一年仅 ~7,000 行（不足 1 MB）。
     - 换来关键精确度：**任意时区偏移下都能精确算出「今天 / 某一天 / 近 7 天 / 近 30 天去重编辑了多少个页面」**（`COUNT(DISTINCT document_id)`），且跨小时多次编辑同一页不会重复计数。
     - `document_id` 不设外键级联删除：即使某篇草稿随后被彻底清空，历史写作产出依然保留。

3. **单一权威写入点**（符合 `AGENTS.md` 数据层唯一判定原则）：
   - **新建文档**：`documents::create_document` 在同一事务中 UPSERT `is_created = true, source = 'manual'`，同步递增进程内 Counter `mica_pages_created_total{source="manual"}`。
   - **导入 / 跨工作区转移**：在 `import` 与 `transfer` 事务中写 `source = 'import' | 'transfer'`，前端趋势图默认仅统计自然创作（可勾选是否包含导入），防止一次导入 500 篇 Vault 把折线图拉爆。
   - **正文编辑**：在 `sync.rs` 持有文档写锁的同一事务内，读取旧 `word_count` 与新 `word_count`：
     - `delta = new_words - old_words`
     - `added = max(delta, 0)`，`deleted = max(-delta, 0)`
     - UPSERT 累加 `words_added`、`words_deleted`、`edit_events + 1`，同步递增进程内 Counter `mica_page_edits_total`、`mica_words_written_total`、`mica_words_deleted_total`。
   - **性能保护**：同一小时内连续高频保存走主键冲突 `ON CONFLICT DO UPDATE`，单行热更新耗时 `< 0.5 ms`。

### 3.2 桌面端本地 SQLite（`crates/mica-core/src/store.rs`）

与云端保持完全同构的设计，升级本地 SQLite schema 版本（保留 `SchemaTooNew` 降级保护）：

1. **`docs` 表新增衍生列** `word_count INTEGER`、`char_count INTEGER`，在现有 `derive_projections_from_doc(&doc)` 单次解码中顺手产出，与 `content_text` / `link_targets` 同步落盘；打开旧库时自动后台/惰性回填。
2. **新增本地活动表 `local_writing_activity_hourly`**：
   ```sql
   CREATE TABLE IF NOT EXISTS local_writing_activity_hourly (
     workspace_id  TEXT    NOT NULL,
     document_id   TEXT    NOT NULL,
     bucket_hour   INTEGER NOT NULL, -- UTC Unix seconds truncated to 3600
     source        TEXT    NOT NULL DEFAULT 'manual',
     is_created    INTEGER NOT NULL DEFAULT 0,
     words_added   INTEGER NOT NULL DEFAULT 0,
     words_deleted INTEGER NOT NULL DEFAULT 0,
     edit_events   INTEGER NOT NULL DEFAULT 0,
     PRIMARY KEY (workspace_id, document_id, bucket_hour)
   );
   ```
3. **本地文件/容量统计**：通过本地 `docs.file_ids`（或已有本地 CAS 索引）关联 `{localDir}/blobs/{sha256}` 文件大小与类型，汇总每个本地工作区的图片数、附件数与已用字节数；云端镜像缓存不计入本地工作区资产，避免重复统计。
4. **共享纯聚合函数**：在 `crates/mica-core/src/usage_stats.rs` 编写纯 Rust 聚合器 `fold_hourly_to_daily(rows, tz_offset_minutes, range_days)`（计算每日序列、补 0、计算 `current_streak` / `longest_streak` / `active_days`），云端 `api-server` 与本地 `mica-core::store` 直接调用同一函数。

---

## 4. 接口契约（HTTP API & 本地 FFI）

云端 HTTP API 与桌面 FFI 返回完全一致的 JSON/DTO 结构，前端一套反序列化逻辑通吃双模式。

### 4.1 总览与工作区明细

- **云端**：`GET /api/stats/overview`（需登录，仅返回当前用户所属工作区，归属 `crates/api-server/src/routes/stats.rs`）
- **本地 FFI**：`local_usage_overview() -> String`（JSON）

响应结构：
```json
{
  "scope": "cloud",
  "generated_at": "2026-11-20T02:30:00Z",
  "workspace_quota_bytes": 5368709120,
  "summary": {
    "workspaces": 30,
    "pages": 13667,
    "folders": 2720,
    "trashed_pages": 1907,
    "empty_pages": 142,
    "words": 42180540,
    "chars": 49597280,
    "images": 5520,
    "attachments": 17,
    "bytes_used": 1070000000
  },
  "health": {
    "page_links": 4820,
    "orphan_pages": 8910,
    "public_shares": 6,
    "open_comment_threads": 3,
    "trashed_pages": 1907
  },
  "workspaces": [
    {
      "id": "36bef1ef-8653-4b28-908c-6466652036eb",
      "name": "greenstor",
      "pages": 240,
      "folders": 28,
      "trashed_pages": 5,
      "words": 684200,
      "chars": 790100,
      "images": 182,
      "attachments": 2,
      "bytes_used": 214748364,
      "quota_bytes": 5368709120,
      "last_updated_at": "2026-11-20T02:08:00Z"
    }
  ]
}
```
> 本地模式下 `workspace_quota_bytes` 与各工作区 `quota_bytes` 为 `null`（无硬配额），`health.public_shares` / `open_comment_threads` 为 `0`。

### 4.2 时间维度趋势与热力图

- **云端**：`GET /api/stats/activity?range=30d&tz_offset=480&workspace_id=<可选>&include_imports=false`
  - `range` 允许值：`1d`（今天）、`7d`、`30d`、`90d`、`365d`
  - `tz_offset`：客户端本地时区相对 UTC 的分钟偏移（整数，范围 `-720..=840`，缺省 `0`）
- **本地 FFI**：`local_usage_activity(range_days: u32, tz_offset_minutes: i32, workspace_id: Option<String>, include_imports: bool) -> String`

响应结构：
```json
{
  "range": "30d",
  "tz_offset_minutes": 480,
  "tracking_since": "2026-11-25T00:00:00Z",
  "totals": {
    "pages_created": 18,
    "pages_edited": 46,
    "words_added": 28450,
    "words_deleted": 6120,
    "net_words": 22330,
    "active_days": 22,
    "current_streak": 6,
    "longest_streak": 14
  },
  "daily": [
    {
      "day": "2026-11-20",
      "pages_created": 2,
      "pages_edited": 5,
      "words_added": 1840,
      "words_deleted": 210,
      "net_words": 1630
    }
  ],
  "heatmap": [
    { "day": "2026-05-25", "score": 3, "words_added": 950, "pages_touched": 4 }
  ]
}
```
- `heatmap` 固定返回近 180 天（桌面宽屏可扩展至 365 天）的每日活跃强度（0–4 五档，与 GitHub 贡献格一致），不受顶部 `7d / 30d` 趋势窗口切换影响，避免用户切换 `今天` 时热力图消失。

---

## 5. Prometheus `/metrics` 扩展与文档规划

### 5.1 设计原则（与现有 [metrics.rs](../../crates/api-server/src/metrics.rs) 一致）

1. **安全边界不变**：继续挂载在根路由 `/metrics`（不经 nginx 代理公网，仅 Compose 内网可见）。
2. **标准时序模型**：存量用 `gauge`，增量活动用进程内单调递增 `counter`；**不暴露 `window="7d"` 或 `user_id` 标签**，所有时间窗口统计通过 PromQL `increase(...[24h])` / `[7d]` / `[30d]` 计算。
3. **复用现有缓存与基数保护**：DB 聚合继续复用 `DB_SNAPSHOT_TTL = 60s`、`mica_workspace_info{workspace_id, name}` join 以及 `mica_workspace_series_truncated` 截断上限，不新增定时扫描压力。

### 5.2 新增指标清单

#### 实例级存量 Gauge
| 指标名 | 类型 | 说明 |
| --- | --- | --- |
| `mica_live_pages_total` | gauge | 全实例活跃文档页数（不含文件夹与回收站） |
| `mica_folders_total` | gauge | 全实例活跃文件夹数 |
| `mica_trashed_pages_total` | gauge | 全实例回收站页面数 |
| `mica_words_total` | gauge | 全实例活跃页面总字数 |
| `mica_chars_total` | gauge | 全实例活跃页面总非空白字符数 |
| `mica_files_total{kind="image\|attachment"}` | gauge | 全实例文件数，按图片/附件区分 |
| `mica_orphan_pages_total` | gauge | 全实例孤岛页面数 |
| `mica_public_shares_total` | gauge | 全实例生效中的公开分享数 |

#### 工作区级存量 Gauge（沿用 `workspace_id` 标签与截断保护）
| 指标名 | 类型 | 说明 |
| --- | --- | --- |
| `mica_workspace_pages{workspace_id}` | gauge | 单工作区活跃页数 |
| `mica_workspace_folders{workspace_id}` | gauge | 单工作区文件夹数 |
| `mica_workspace_words{workspace_id}` | gauge | 单工作区活跃页总字数 |
| `mica_workspace_files{workspace_id,kind}` | gauge | 单工作区图片/附件数量（`kind="image"\|"attachment"`） |
| `mica_workspace_bytes_used{workspace_id}` | gauge | *已有*：单工作区已用存储字节 |
| `mica_workspace_quota_bytes` | gauge | *已有*：单工作区配额上限 |

#### 写作活动单调 Counter（供 PromQL 计算 1d / 7d / 30d 趋势）
| 指标名 | 类型 | 说明 |
| --- | --- | --- |
| `mica_pages_created_total{source="manual\|import\|transfer"}` | counter | 新建页面累计计数 |
| `mica_page_edits_total` | counter | 文档正文写入累计次数 |
| `mica_words_written_total` | counter | 正文新增字数累计计数 |
| `mica_words_deleted_total` | counter | 正文删减字数累计计数 |

> **PromQL 示例**（后续完整写入 `docs/ops/metrics.md`）：
> - 近 7 天新增字数：`sum(increase(mica_words_written_total[7d]))`
> - 近 24 小时手动新建页数：`increase(mica_pages_created_total{source="manual"}[24h])`
> - 按名称查看各工作区字数 Top 10：`topk(10, mica_workspace_words * on(workspace_id) group_left(name) mica_workspace_info)`
> - 单工作区容量超过 80% 告警：`mica_workspace_bytes_used / on() group_left() mica_workspace_quota_bytes > 0.8`

### 5.3 独立监控文档规划：`docs/ops/metrics.md`

实施时同步新增 `docs/ops/metrics.md`（并在 `docs/README.md` 与 `deploy/monitoring/README.md` 链接），包含：
1. `/metrics` 架构定位、网络暴露边界与 60 秒快照缓存机制；
2. 完整指标字典（HTTP / CRDT / DB Pool / Blob GC / Process / 知识库容量与活动全量表格）；
3. 常用 PromQL 查询集合（1d / 7d / 30d 写作趋势、Top 工作区、容量增长预测 `predict_linear`）；
4. 推荐告警规则（容量 80% / CRDT 完整性失败 / DB 获取延迟 / Blob GC 失败）与最小接入配置。

---

## 6. UI / 交互设计

### 6.1 模块边界（遵守代码大小棘轮）

- 新增独立库 **`clients/mica_flutter/lib/ui/usage_stats_panel.dart`**（含 KPI 卡片、轻量自绘 `CustomPainter` 趋势图、热力图、工作区排序表），**不向 `main.dart` 或 `settings_dialog.dart` 堆逻辑**。
- 图表使用 Flutter 原生 `CustomPainter` 自绘（柱状/折线趋势图 + 52×7 / 26×7 热力图网格 + Hover Tooltip），**不引入重型第三方图表库**，保持依赖最小化与浅/深色主题统一。
- 严格遵循 Mica 视觉规范：分段切换统一使用 `MicaSegmentGroup` / `MicaSegmentButton`（满足 `segmented_choice_test.dart`），主界面保持冷静中性色阶（满足 `blue_accent_policy_test.dart`）。

### 6.2 面板布局（位于 `MicaSettingsShell` 右侧内容区，宽约 820px）

```text
┌──────────────────────────────────────────────────────────────────────────┐
│ 使用统计                                                                  │
│ 知识库规模、存储使用与写作活跃趋势                   [工作区: 全部 ▾] [刷新] │
├──────────────────────────────────────────────────────────────────────────┤
│ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────────┐ │
│ │ 工作区   │ │ 活跃页面 │ │ 总字数   │ │ 图片/附件│ │ 已用存储          │ │
│ │   30     │ │  13,667  │ │ 4,218 万 │ │5,520 / 17│ │ 1.07 GB          │ │
│ │ 文件夹2720│ │ 篇均3086字│ │ 4960万字符│ │          │ │ 单区上限 5 GB    │ │
│ └──────────┘ └──────────┘ └──────────┘ └──────────┘ └──────────────────┘ │
│                                                                          │
│ 写作趋势    [今天 | 近 7 天 | 近 30 天 | 近 90 天 | 近 1 年]                │
│             [新建页面 | 编辑页面 | 新增字数]           ☐ 含导入/转移       │
│ ┌──────────────────────────────────────────────────────────────────────┐ │
│ │  新建 18 页 · 编辑 46 页 · 新增 28,450 字 · 连续活跃 6 天              │ │
│ │  ▁▂▄▃▆▅▇▃▂▅▆▄▃▂▁▃▅▇▆▄▃▂▄▅▃▂▁▄▆▅   (支持 Hover 查看当日精确明细)        │ │
│ └──────────────────────────────────────────────────────────────────────┘ │
│                                                                          │
│ 活跃热力图（近半年）                                活跃 128 天 · 最长 14 天│
│ ░▒▓█▒░░▒▓██▓▒░▒▓▓█▒░░▒▓█▓▒░▒▓▓█▒░  (悬停显示日期、字数与触达页数)         │
│                                                                          │
│ 知识库健康度                                                             │
│ [双向链接 4,820] [孤岛页面 8,910 (65%)] [空页面 142] [回收站 1,907] [分享 6]│
│                                                                          │
│ 各工作区明细（点击工作区行可按该工作区筛选上方趋势）                        │
│ 工作区名         页面 ▾   字数     图片/附件   已用容量 / 配额   占比     │
│ ──────────────────────────────────────────────────────────────────────── │
│ greenstor         240    68.4 万   182 / 2    205 MB / 5 GB   [██░░░ 4%] │
│ storage           812   241.0 万   610 / 4    312 MB / 5 GB   [███░░ 6%] │
│ ...                                                                      │
└──────────────────────────────────────────────────────────────────────────┘
```

### 6.3 异步与边界处理（对齐 [async-review-checklist.md](../dev/async-review-checklist.md)）

- 请求绑定账号/服务器/模式代际与筛选条件（`workspaceId` + `range`），返回时校验归属，防止快速切换时间窗口导致旧响应覆盖新结果。
- 弱网或离线打开云端统计时，保留清晰错误态与「重试」按钮；桌面离线切换到本地模式时直接读本地 FFI 毫秒级呈现。

---

## 7. 分阶段实施计划与验收清单

| 阶段 | 交付内容 | 核心验收标准 |
| --- | --- | --- |
| **M1. 单一字数引擎与存储基建** | `crates/mica-core/src/word_count.rs` + `usage_stats.rs`（小时桶折叠与 streak 计算）；Postgres `0040_usage_stats.sql` + `sync.rs` co-write 与回填；本地 SQLite schema 升级与 `derive_projections_from_doc` 扩展 | Rust 与 Dart 字数统计对中/英/混排/标点黄金样例 100% 一致；真实 Postgres 迁移 + 回填测试通过；小时桶在 `+480` / `-300` / `0` 跨日折叠单测通过 |
| **M2. 云端 API 与本地 FFI 同构接口** | `crates/api-server/src/routes/stats.rs`（`/api/stats/overview`、`/api/stats/activity`）；桌面 FFI 暴露 `local_usage_overview` / `local_usage_activity`；`ApiClient` 与本地适配器接线 | 真实 Postgres 集成测试覆盖权限隔离（仅可见所属工作区）、导入排除、去重编辑页数；本地 FFI 单测验证与云端输出口径一致 |
| **M3. Prometheus `/metrics` 与文档** | 扩展 `crates/api-server/src/metrics.rs` 存量 Gauge 与活动 Counter；新增 `docs/ops/metrics.md` 并更新 `docs/README.md` 与 `deploy/monitoring/README.md` | `metrics.rs` 单测覆盖新指标渲染格式、0 值系列与截断保护；`cargo clippy -- -D warnings` 全绿 |
| **M4. 设置页 UI 与端到端验收** | 新建 `lib/ui/usage_stats_panel.dart`；`SettingsDialog` 新增 `initialSection` 与「使用统计」页签；`AccountMenu` 增加入口；补齐中英文 `arb` | Widget 测试覆盖云端/本地双模式、空态、排序与跨窗口竞态；通过 `check-file-size.sh`、`blue_accent_policy_test`、`segmented_choice_test`；Web 端 Playwright 截图核验浅/深色与宽/窄窗口 |
