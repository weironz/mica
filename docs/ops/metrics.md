# Mica Prometheus 指标与监控参考

> 对应实现：[crates/api-server/src/metrics.rs](../../crates/api-server/src/metrics.rs) · 监控示例栈：[deploy/monitoring/README.md](../../deploy/monitoring/README.md) · 使用统计设计：[docs/features/usage-statistics.md](../features/usage-statistics.md)

本文档是 `mica-api-server` 暴露的 `/metrics` 端点的权威参考，覆盖采集架构、网络暴露边界、完整指标字典、常用 PromQL 查询与推荐告警规则。

---

## 1. 架构与网络暴露边界

- **端点路径**：`GET /metrics`（挂载在根路由，**不**在 `/api` 前缀下），返回 Prometheus text exposition format `0.0.4`（`text/plain; version=0.0.4; charset=utf-8`）。
- **无第三方指标库依赖**：指标注册表与直方图在 `crates/api-server/src/metrics.rs` 中零依赖手写，避免高基数标签泄露与依赖膨胀。
- **结构性网络隔离**：生产 `deploy/nginx.conf` 仅将 `/api/`、`/ws/`、`/s/`、`/reset-password`、`/verify-email` 反向代理至后端，公网请求 `GET /metrics` 会落入 SPA fallback 返回 `index.html`，绝不泄露任何 `mica_*` 序列。Prometheus 通过 Docker Compose 内部网络直接抓取 `http://api:8080/metrics`。
  > 如果未来 nginx 放宽代理规则，必须同步为 `/metrics` 增加鉴权或显式拒绝公网访问。
- **数据库快照缓存（60 秒 TTL）**：所有存量业务与容量指标（`DbSnapshot`）均复用 `DB_SNAPSHOT_TTL = 60s` 内存缓存；单次抓取不会触发全表扫描。若数据库短暂不可用，自动回退上一次成功快照，避免监控面板误报归零。
- **高基数保护**：
  - HTTP `method` 仅保留标准 7 动词（`GET`/`HEAD`/`POST`/`PUT`/`PATCH`/`DELETE`/`OPTIONS`），其余归入 `OTHER`。
  - HTTP `route` 使用 Axum `MatchedPath` 模板，永不带入原始 UUID 路径。
  - 每工作区序列按存储容量降序最多输出 `MAX_WORKSPACE_SERIES = 1000` 个工作区，超出部分由 `mica_workspace_series_truncated` 诚实报告。
  - **绝不**在 Prometheus 中暴露 `user_id × window` 等个人明细维度（用户级明细走鉴权 HTTP API `/api/stats/*`）。

---

## 2. 完整指标字典

### 2.1 运行状态与构建信息

| 指标名 | 类型 | 标签 | 说明 |
| --- | --- | --- | --- |
| `mica_build_info` | Gauge | `version` | 当前运行的 `mica-api-server` 版本号（恒为 `1`） |
| `mica_process_resident_bytes` | Gauge | — | 进程常驻内存 RSS 字节数（Linux `/proc/self/status`） |
| `mica_process_open_fds` | Gauge | — | 进程当前打开文件描述符数（Linux `/proc/self/fd`） |

### 2.2 HTTP、WebSocket 与数据库连接池

| 指标名 | 类型 | 标签 | 说明 |
| --- | --- | --- | --- |
| `mica_http_requests_total` | Counter | `method`, `route`, `status` | HTTP 请求累计总数 |
| `mica_http_request_duration_seconds` | Histogram | `method`, `route` | HTTP 请求延迟分布（12 桶：`5ms`..`10s`、`+Inf`） |
| `mica_http_in_flight_requests` | Gauge | — | 当前正在处理的 HTTP 请求数 |
| `mica_ws_connections` | Gauge | — | 当前活跃文档 WebSocket 连接数 |
| `mica_db_pool_connections` | Gauge | `state="idle\|in_use"` | Postgres 连接池状态分布 |
| `mica_db_acquire_probe_seconds` | Histogram | — | 每 15 秒探测一次连接池 `acquire()` 的真实等待耗时 |

### 2.3 CRDT 同步与完整性红线

| 指标名 | 类型 | 标签 | 说明 |
| --- | --- | --- | --- |
| `mica_crdt_integrity_failures_total` | Counter | `stage` | CRDT 完整性校验拒绝次数（刻意保留 0 值基线，非 0 即高优排查） |
| `mica_crdt_push_duration_seconds` | Histogram | — | 单次 `sync.push` 校验、折叠与落库耗时 |
| `mica_crdt_push_bytes_total` | Counter | — | 接收的 CRDT 更新总字节数 |
| `mica_crdt_push_rejected_total` | Counter | — | 服务端拒绝的 CRDT 推送次数 |
| `mica_crdt_lag_notices_total` | Counter | — | 通知客户端落出广播窗口需重新拉取的次数 |

### 2.4 实例存量规模与知识库健康度（60s DB 快照）

| 指标名 | 类型 | 标签 | 说明 |
| --- | --- | --- | --- |
| `mica_users_total` | Gauge | — | 未注销注册用户总数 |
| `mica_workspaces_total` | Gauge | — | 工作区总数 |
| `mica_documents_total` | Gauge | — | `documents` 表总行数（含回收站） |
| `mica_live_pages_total` | Gauge | — | 活跃页面数（`object_type='document' AND is_deleted=false`） |
| `mica_folders_total` | Gauge | — | 活跃文件夹数（`object_type='folder' AND is_deleted=false`） |
| `mica_trashed_pages_total` | Gauge | — | 回收站内页面数 |
| `mica_words_total` | Gauge | — | 全实例活跃页面总字数（与 `mica-core::word_count` 同口径） |
| `mica_chars_total` | Gauge | — | 全实例活跃页面非空白总字符数 |
| `mica_files_total` | Gauge | `kind="image\|attachment"` | 全实例已上传图片与附件数量 |
| `mica_orphan_pages_total` | Gauge | — | 既无入链也无出链的孤岛活跃页面数 |
| `mica_public_shares_total` | Gauge | — | 当前未撤销的公开分享页数量 |
| `mica_storage_bytes` | Gauge | `scope="total"` | 全实例已占用存储总字节数（已注册文件 + 未注册预占） |

### 2.5 工作区维度容量与资产分布（60s DB 快照）

| 指标名 | 类型 | 标签 | 说明 |
| --- | --- | --- | --- |
| `mica_workspace_info` | Gauge | `workspace_id`, `name` | 工作区 ID → 名称映射（恒为 `1`，用于 PromQL `group_left(name)` 关联，避免改名断序列） |
| `mica_workspace_bytes_used` | Gauge | `workspace_id` | 单工作区已用存储字节数（与上传配额拦截同源） |
| `mica_workspace_quota_bytes` | Gauge | — | 当前生效的单工作区配额上限（`MICA_WORKSPACE_QUOTA_BYTES`） |
| `mica_workspace_pages` | Gauge | `workspace_id` | 单工作区活跃页面数 |
| `mica_workspace_folders` | Gauge | `workspace_id` | 单工作区活跃文件夹数 |
| `mica_workspace_words` | Gauge | `workspace_id` | 单工作区活跃页面总字数 |
| `mica_workspace_files` | Gauge | `workspace_id`, `kind="image\|attachment"` | 单工作区图片 / 附件文件数 |
| `mica_workspace_series_truncated` | Gauge | — | 因超过 `MAX_WORKSPACE_SERIES`（1000）而被截断的工作区数量 |

### 2.6 写作活跃增量计数器（进程级单调递增）

| 指标名 | 类型 | 标签 | 说明 |
| --- | --- | --- | --- |
| `mica_pages_created_total` | Counter | `source="manual\|import\|transfer"` | 进程启动以来新建页面数（区分手动新建、导入、跨工作区转移；0 值序列始终输出） |
| `mica_page_edits_total` | Counter | — | 进程启动以来成功接受的文档编辑次数（含 WS `sync.push` 与 REST/MCP 写入） |
| `mica_words_written_total` | Counter | — | 进程启动以来累计新增字数（每次保存 `max(new - old, 0)`） |
| `mica_words_deleted_total` | Counter | — | 进程启动以来累计删除字数（每次保存 `max(old - new, 0)`） |

### 2.7 对象存储 Blob GC

| 指标名 | 类型 | 标签 | 说明 |
| --- | --- | --- | --- |
| `mica_blob_gc_sweeps_total` | Counter | — | 已完成的 Blob 垃圾回收轮次 |
| `mica_blob_gc_objects_total` | Counter | `op="scanned\|deleted"` | 扫描与清理的孤儿对象数 |
| `mica_blob_gc_bytes_freed_total` | Counter | — | 累计回收字节数 |
| `mica_blob_gc_failures_total` | Counter | — | 失败的 GC 轮次 |
| `mica_blob_gc_eligible_orphan_objects` | Gauge | — | 上一轮已过宽限期的待回收对象数 |
| `mica_blob_gc_eligible_orphan_bytes` | Gauge | — | 上一轮已过宽限期的待回收字节数 |

---

## 3. 常用 PromQL 查询配方

```promql
# 1. 各工作区配额占用率 Top 10（附带可读名称）
topk(10,
  (mica_workspace_bytes_used / on() group_left() mica_workspace_quota_bytes)
  * on(workspace_id) group_left(name) mica_workspace_info
)

# 2. 近 24 小时手动新建页面数（不含导入/转移）
increase(mica_pages_created_total{source="manual"}[24h])

# 3. 近 7 天累计新增字数与净增字数
increase(mica_words_written_total[7d])
increase(mica_words_written_total[7d]) - increase(mica_words_deleted_total[7d])

# 4. 孤岛页面占比
mica_orphan_pages_total / clamp_min(mica_live_pages_total, 1)

# 5. API p95 延迟（按路由）
histogram_quantile(0.95,
  sum by (le, route) (rate(mica_http_request_duration_seconds_bucket[5m]))
)

# 6. 数据库连接池获取耗时 p95（08-03 IO 饥饿故障先行指标）
histogram_quantile(0.95, sum by (le) (rate(mica_db_acquire_probe_seconds_bucket[5m])))

# 7. CRDT 单次推送 p95 耗时
histogram_quantile(0.95, sum by (le) (rate(mica_crdt_push_duration_seconds_bucket[5m])))
```

---

## 4. 推荐告警规则参考

> 按需复制到你自己的 Prometheus `alert.rules.yml`；未在默认 compose 中强行启用以免无值守渠道时产生噪音。

```yaml
groups:
  - name: mica-core
    rules:
      - alert: MicaCrdtIntegrityFailure
        expr: increase(mica_crdt_integrity_failures_total[5m]) > 0
        for: 0m
        labels: { severity: critical }
        annotations:
          summary: "Mica CRDT integrity failure detected"

      - alert: MicaDbAcquireSlow
        expr: histogram_quantile(0.95, sum by (le) (rate(mica_db_acquire_probe_seconds_bucket[5m]))) > 0.5
        for: 2m
        labels: { severity: warning }
        annotations:
          summary: "Postgres pool acquire p95 > 500ms (IO or connection starvation)"

      - alert: MicaWorkspaceQuotaHigh
        expr: mica_workspace_bytes_used / on() group_left() mica_workspace_quota_bytes > 0.85
        for: 15m
        labels: { severity: warning }
        annotations:
          summary: "Workspace {{ $labels.workspace_id }} exceeded 85% storage quota"

      - alert: MicaHighHttp5xxRate
        expr: sum(rate(mica_http_requests_total{status=~"5.."}[5m])) / clamp_min(sum(rate(mica_http_requests_total[5m])), 0.01) > 0.02
        for: 5m
        labels: { severity: warning }
        annotations:
          summary: "Mica HTTP 5xx ratio exceeded 2%"
```
