# 全栈纯 Rust（L1+L2）Release 性能基线与优化实录（2026-11-25）

- **测试环境**：Windows x64，Release Profile（`cargo run -p mica-bench --release`）
- **原始快照**：标准基线 NDJSON 见 [2026-11-25-standard.ndjson](2026-11-25-standard.ndjson)
- **复现命令**：`just bench`（标准基线）、`just bench-10k`（万篇知识库重压）

---

## 一、首轮基准抓出的两类真实瓶颈与优化对比

### 1. 万篇知识库 SQLite 链接边聚合（`mica-core`）
- **根因**：`store_stats::usage_overview` 与 `store::graph_local` 使用 `JOIN json_each(s.link_targets) j JOIN local_view tgt`。SQLite 优化器将 `tgt` 重排到 `json_each` 之前，退化为 `O(N²)` 笛卡尔扫描；同时批量投影刷新缺少外层 `SAVEPOINT` 事务且 `reindex_doc` 在健康路径解码 `MicaDoc` 两遍。
- **修复**：`CROSS JOIN local_view tgt` 固定主键 `(origin, id)` 点查顺序（附 `EXPLAIN QUERY PLAN` 回归测试）+ `with_savepoint` 批量事务 + 消除 `reindex_doc` 冗余解码。

| 指标 | 优化前 | 优化后 | 提升 |
| --- | ---: | ---: | ---: |
| `usage_stats_sql_aggregation`（1,000 篇，2,000 边） | `664.74 ms` | **`10.95 ms`** | **60×** |
| `usage_stats_sql_aggregation`（10,000 篇，20,000 边） | **`> 196,000 ms`（超时卡死）** | **`544.50 ms`** | **> 360×** |

### 2. 2,000 块超长文档编辑器热路径（`mica-core` + `mica-editor`）
- **根因**：
  1. `EditorSession::ensure_root_in_doc()` 每次编辑都调用 `MicaDoc::to_blocks()` 全量反序列化全部 2,000 块 yrs CRDT；
  2. `EditorSession::dispatch()` 每次命令开头无条件深拷贝 `self.blocks.clone()` 并在结尾做全量 `diff_block_patches`；
  3. `MicaDoc::find_parent()` 在删除/移动/分块/合并时遍历全部 2,000 个块查找父节点；
  4. `cmd_undo_redo` / `cmd_apply_remote_update` 未接入 `MicaDoc::track_changes()` 增量读取。
- **修复**：新增 `MicaDoc::has_block()`（O(1)）、`find_parent` 优先探测 `root`、`dispatch` 改为按 `ctx.ops` O(|ops|) 产出 `dirty_blocks`（零整表 clone）、`Undo`/`Redo`/`ApplyRemoteUpdate` 接入 `take_changed_block_ids()` + `blocks_by_id()` 增量快路径。

| 指标（2,000 块 / 302 KiB 混排文档） | 优化前 | 优化后 | 提升 |
| --- | ---: | ---: | ---: |
| `typing_insert_text (300 ops)` | `p50=24.19 ms / p99=33.30 ms` | **`p50=69.8 µs / p99=86.6 µs`** | **346×** |
| `split_block (Enter, 100 ops)` | `p50=25.92 ms / p99=31.33 ms` | **`p50=259.8 µs / p99=342.3 µs`** | **100×** |
| `merge_block (Backspace, 100 ops)` | `p50=26.17 ms / p99=32.40 ms` | **`p50=383.0 µs / p99=429.7 µs`** | **68×** |
| `typing_undo_redo (50 pairs)` | `~35.5 ms` | **`undo 1.21 ms / redo 1.20 ms`** | **29×** |
| `undo / redo (50 pairs, 结构变更)` | `undo 35.46 ms / redo 35.66 ms` | **`undo 25.72 ms / redo 25.44 ms`** | **1.4×** |
| `in_page_find_all ('Rust', 1167 hits)` | `11.64 ms` | **`2.84 ms`** | **4.1×** |
| `remote_crdt_merge (500 updates)` | `p50=20.2 µs` | **`p50=15.4 µs`** | **1.3×** |

---

## 二、优化后完整基线数据

### 1. 超长混排文档（2,000 块，301.7 KiB）
| 指标 | 标准基线 | 万篇同步run |
| --- | ---: | ---: |
| `parse_markdown` | `18.28 ms` | `17.70 ms` |
| `session_from_markdown (CRDT)` | `131.63 ms` | `129.50 ms` |
| `typing_insert_text (300 ops)` | `p50=69.8 µs / p95=75.4 µs / p99=86.6 µs` | `p50=69.9 µs / p95=77.0 µs / p99=81.8 µs` |
| `typing_undo_redo (50 pairs)` | `undo 1.21 ms / redo 1.20 ms` | `undo 1.18 ms / redo 1.18 ms` |
| `split_block (Enter, 100 ops)` | `p50=259.8 µs / p99=342.3 µs` | `p50=258.3 µs / p99=306.5 µs` |
| `merge_block (Backspace, 100 ops)` | `p50=383.0 µs / p99=429.7 µs` | `p50=385.4 µs / p99=430.1 µs` |
| `undo / redo (50 pairs, 结构变更)` | `undo 25.72 ms / redo 25.44 ms` | `undo 24.81 ms / redo 25.96 ms` |
| `in_page_find_all ('Rust')` | `2.84 ms` | `2.90 ms` |
| `export_markdown` | `17.44 ms` | `17.39 ms` |
| `export_html` | `16.82 ms` | `15.99 ms` |

### 2. CRDT 编辑轨迹回放 & 双端协同合并
| 指标 | 20,000 步 | 50,000 步 |
| --- | ---: | ---: |
| `replay_throughput` | `319.91 ms`（62,517 ops/s） | `2,452.29 ms`（20,389 ops/s） |
| `step_latency (overall)` | `p50=8.90 µs / p95=53.90 µs / p99=72.00 µs` | `p50=29.30 µs / p95=158.90 µs / p99=208.80 µs` |
| `early vs late (2000-op)` | `2.00 µs → 35.90 µs`（17.95×） | `1.90 µs → 116.60 µs`（61.37×） |
| `state_size & cold_decode` | 17.6 KB / yrs 52.2 KB（2.96×），解码 `2.02 ms` | 43.1 KB / yrs 138.2 KB（3.21×），解码 `5.31 ms` |
| `remote_crdt_merge (500 updates)` | `p50=15.4 µs / p99=27.9 µs` | `p50=15.4 µs / p99=22.3 µs` |

### 3. 大规模合成知识库（真实磁盘 SQLite Vault）
| 指标 | 1,000 篇 + 100 目录 | 10,000 篇 + 1,000 目录 |
| --- | ---: | ---: |
| `sqlite_vault_bulk_ingest` | `702.2 ms`（1,424 docs/s，DB 3.39 MiB） | `8,766.5 ms`（1,141 docs/s，DB 33.14 MiB） |
| `cold_open_vault_and_tree` | `7.27 ms`（top 5 / live 1,100） | `69.75 ms`（top 5 / live 11,000） |
| `sqlite_random_doc_load (200 reads)` | `p50=120.1 µs / p99=191.8 µs` | `p50=126.9 µs / p99=192.0 µs` |
| `search_common_hit` | `0.45 ms` | `0.45 ms` |
| `search_rare_needle` | `3.03 ms` | `30.69 ms` |
| `search_worst_case_miss` | `3.14 ms` | `37.41 ms` |
| `build_backlink_graph` | `1.35 ms`（2,000 边） | `13.32 ms`（20,000 边） |
| `usage_stats_sql_aggregation` | `10.95 ms` | `544.50 ms` |

### 4. 大型无限白板 / 思维导图（501 节点）
| 指标 | 实测结果 |
| --- | ---: |
| `board_load_and_auto_layout` | 加载 `1.68 ~ 1.74 ms`，自动布局 `2.22 ~ 2.24 ms` |
| `board_svg_merge_and_search_text` | SVG `3.66 ~ 3.78 ms`（206.9 KiB），三方合并 `5.69 ~ 5.74 ms`，文本提取 `1.28 ~ 1.32 ms` |

---

## 三、套件 ①：Windows 原生桌面端实测（`mica-rust.exe` Release + L3 NSH 壳）

- **进程冷启动就绪（5 次独立临时隔离配置，含 SQLite Vault + CRDT 加载 + 首帧探针 `MICA_STARTUP_PROBE_FILE`）**：
  - 5 次实测：`330.2 ms`、`250.8 ms`、`293.0 ms`、`247.5 ms`、`281.5 ms` → **中位数 `281.5 ms`**（`min=247.5 ms / max=330.2 ms`）。
- **L3 原生 UI 壳（NSH 1280×820 Release，`l3_frame_bench`）**：
  - 首帧挂载 `shell_mount_first_frame`：**`19.24 ms`**。
  - 短文档 vs 100+ 块虚拟列表连续打字（均含 `nsh-testing::Headless` 单步 ~100 ms 固定 settle 等待）：短文档 `p50=103.33 ms`，100 块文档 `p50=105.69 ms`，Enter 分块 `p50=106.67 ms`，Undo `p50=105.90 ms`，打开大纲 `103.61 ms`——扣除 harness 固定 settle 后单帧重绘计算仅约 `3~6 ms`，且 100 块文档相对短文档几乎零退化（`+2.36 ms`）。
- **黑盒屏幕像素法（[desktop_latency.ps1](../desktop_latency.ps1)，真实解锁 Windows 桌面 DWM `SendInput → BitBlt`，`1280×820` 窗口 / `819×574` 采样 ROI）**：
  - 本次实测冷启动：**`279.1 ms`**（与 5 次冷启中位数 `281.5 ms` 基本一致）。
  - 全部采样 `35/35` 命中、`0` 超时（含 GDI `BitBlt` + `GetDIBits` 全 ROI 轮询开销与 Windows DWM 合成帧等待）：

| 场景 | 命中 / 超时 | `min` | `p50` | `p95` | `p99` | `max` |
| --- | :---: | ---: | ---: | ---: | ---: | ---: |
| **短文档连续打字** | `35/35` (`0`) | `19.70 ms` | **`68.74 ms`** | `84.72 ms` | `85.75 ms` | `85.75 ms` |
| **1,000 块粘贴长文档打字** | `35/35` (`0`) | `19.27 ms` | **`70.21 ms`** | `417.21 ms` | `417.96 ms` | `417.96 ms` |

  - **解读**：短文档与 1,000 块长文档的中位真屏幕延迟几乎一致（`68.74 ms` vs `70.21 ms`，仅 **`+1.47 ms`**，最优单帧 **`19.27 ms`**），说明虚拟列表与增量渲染在长文常态打字下没有退化；长文 35 次采样中出现 1–2 次 `~417 ms` 尾刺（推测与粘贴/热身后的 300 ms 防抖落盘周期触发主线程 `take_pending_markdown` + `compute_backlinks_for` 或偶发换行滚屏有关，已记录作为后续优化线索）。

---

## 四、套件 ③：WebAssembly 浏览器自动化性能门禁（Playwright + Chromium CDP）

基于最新 `scripts/build-web-rust.sh` 构建产物（`mica_rust_bg.wasm` raw `13.36 MiB` / Brotli-11 `3.65 MiB`，首屏总包 Brotli-11 **`3.75 MiB <= 4.0 MiB`** 硬门禁；懒加载 `math.wasm` gzip-9 `979.8 KiB`、`mermaid.wasm` gzip-9 `1.92 MiB`），在 Chromium `1280×820` 实测：

| 阶段 | 指标 | 实测结果 |
| --- | --- | ---: |
| **冷启动** | `dom_ready_ms` / `wasm_download` / `first_interactive_ms` | `14.67 ms` / `417.25 ms` / **`480.68 ms`** |
| **冷启动** | `login_to_synced_ms`（登录 → WS `已实时同步`） | **`228.66 ms`** |
| **冷启动** | 初始内存 & DOM 规模 | JS 堆 **`3.9 MiB`**，`874` DOM 节点 |
| **2,000 块文档虚拟滚动** | 切页打开首块可见 `open_large_doc_ms` | `516.79 ms` |
| **2,000 块文档虚拟滚动** | 真实滚轮帧率（185 帧采样） | **均值 `59.7 FPS` / P95 低位 `59.9 FPS`** |
| **2,000 块文档虚拟滚动** | 帧间隔 & 长帧卡顿（>50ms） | `p50=16.7 ms` / `p99=16.8 ms` / `max=33.4 ms`，**0 帧卡顿（0%）** |
| **2,000 块文档真机打字** | `key_to_paint`（40 键，2×rAF 含绘制完成） | **`p50=10.3 ms` / `p95=17.7 ms` / `p99=17.9 ms`** |
| **50 次反复切页稳定性** | 切页耗时 `p50 / p95 / p99` | `116.39 ms` / `150.01 ms` / `210.75 ms` |
| **50 次反复切页稳定性** | 显式 GC 后 JS 堆内存斜率 & DOM 节点变化 | `4.96 → 5.25 MiB`（**`+0.28 MiB`，`5.79 KiB/次`**），DOM `2,081 → 1,963`（**-118，零泄漏**） |

---

## 五、套件 ④：服务端真实协议 `/ws/` 协作压测（`mica-ws-load` + Release `mica-api-server` + Postgres 18）

独立压测数据库 `mica_bench_perf`（`DATABASE_MAX_CONNECTIONS=25`），真实说话 `/ws/workspaces/{ws}/documents/{doc}?v=1` 协议（`sync.bootstrap` / `sync.push` / `sync.pull` / `sync.update` / `sync.ack`）：

| 场景 | 规模 | Push-to-Ack 延迟 / 吞吐 | Peer-to-Peer 广播延迟 & 扇出 | 收敛与错误 |
| --- | --- | ---: | ---: | --- |
| **热点房间（Hot Room）** | 30 并发编辑同一文档 × 8 pushes（240 写） | `p50=141.41 ms` / `p95=188.50 ms` / `p99=202.53 ms`（**`195.6 pushes/s`**） | `p50=207.77 ms` / `p95=383.76 ms` / `p99=455.04 ms`（**6,960 次扇出**） | **`converged=true`**，`lags=0`，`errs=0` |
| **多房间并发（Multi-Room）** | 10 房 × 5 人 = 50 并发 WS × 6 pushes（300 写） | **`p50=33.97 ms` / `p95=38.90 ms` / `p99=44.81 ms`（`1,205.0 pushes/s`）** | **`p50=49.96 ms` / `p95=77.73 ms` / `p99=90.86 ms`**（1,200 次扇出） | **`all_converged=true`**，`lags=0`，`errs=0` |
| **断网重连风暴（Reconnect Storm）** | 30 人断线各攒 5 步离线编辑后同时重连同步 | 全员收敛总耗时 **`736.4 ms`**（单端 `p50=667.90 ms` / `p99=735.56 ms`） | `sync.pull` + `sync.push` 并发突发 | **`all_edits_preserved=true`**，`errs=0` |

**服务端 `/metrics` 压测后快照**：
- `mica_crdt_integrity_failures_total{kind="push"} = 0`
- `mica_crdt_lag_notices_total = 0`
- `mica_crdt_push_rejected_total = 0`
- `mica_ws_connections = 0`（压测结束后零连接泄漏）
- `mica_db_pool_connections{state="idle"} = 23` / `in_use = 2`

