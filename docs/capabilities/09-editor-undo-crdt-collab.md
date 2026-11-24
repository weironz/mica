# 09 — 撤销/重做、实时协作与远端合并

> **适用范围**：桌面端、Web、移动端。

## 核心不变量

1. **单一 CRDT 文档核心与单历史栈**：每篇文档底层唯一由 `mica_core::MicaDoc`（`yrs`）承载；撤销（`Mod+Z`）与重做（`Mod+Y` / `Mod+Shift+Z`）必须兼容协作前向操作语义，绝不把远端协作者的修改一并撤销掉。
2. **本地未提交编辑受代际保护**：远端快照或增量更新到达时，严禁覆盖用户正在输入且尚未确认落盘的本地块文本与 marks（文本与 marks 偏移必须成对保留或成对采用，绝不可混拼）。
3. **异步提交绑定归属与代际**：跨 `await` 回写共享状态或持久化文档时，必须校验当前账号/服务器源/页面 ID 与编辑代际（遵循 `docs/dev/async-review-checklist.md`），防止切页慢请求写串页面。

---

## 能力与交互规格

### 撤销与重做
- **`CAP-09-01` 400ms 输入聚合与结构边界切分**：连续打字在静默窗口（约 `400ms`）内合并为一个撤销批次；遇到回车拆块、粘贴、删除块、切换块类型、表格结构操作、AI 批量改写等结构操作时立即切分批次，确保一键撤销精确回退一步。
- **`CAP-09-02` 全操作可撤销**：普通文本编辑、格式 toggle、块删除（含代码块 `(0,0)` 删除与框选批量删除）、底部自动追加行、表格单元格编辑与行列增删、粘贴、AI 块级改写，均进入统一 Undo/Redo 栈并正确维护 `can_undo` / `can_redo` 按钮灰显状态。

### CRDT 实时协作与远端合并
- **`CAP-09-03` 增量 yrs 更新广播与接收**：本地编辑输出最小 `yrs` v1 增量 update，通过 WebSocket 与离线 outbox 同步到云端；远端 peer 的 update 应用时不回声触发本地持久化死循环。
- **`CAP-09-04` 远端合并不丢光标（`SelectionReconcile`）**：
  - 远端在光标所在块之前插入/删除块，或在光标所在块内修改文字时，按 `block_id` 与文本 diff 自动重映射本地选区起止偏移。
  - 若远端删除了本地光标所在的块，按旧文档邻接关系优先跳到后继存活块开头，再回退到前驱存活块末尾（不机械停留在已移位的旧数字下标）。
- **`CAP-09-05` 离线优先与失败重试**：断网时所有本地编辑照常写入本地 SQLite / IndexedDB 持久化与同步 outbox；网络恢复后按指数退避自动重连重放，写队列失败必须明确向用户报错而非静默吞掉编辑。

---

## 历史踩坑与反例

- **反例 1（双撤销栈互相打架）**：当 L3 编辑器控件自带一套本地撤销栈、L2 `EditorSession` 又维护一套 CRDT 撤销栈时，`Mod+Z` 会出现回退不同步甚至文档错乱。必须在单栈 `EditorSession` 统一管理。
- **反例 2（文本留本地、marks 用远端导致格式错位）**：远端合并时保留了本地文本却无条件采纳了远端 `data.marks`，因为 marks 是字符偏移，导致加粗和链接凭空漂移到错误文字上。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-09-01` / `02` 单栈 Undo/Redo 与全操作覆盖 | P0 | 全端 | ✅ | `mica-editor::insert_text_split_and_backspace_merge_with_undo_redo` + `ai_block_edits_apply_atomically_and_reject_stale_snapshot` |
| `CAP-09-03` 增量 yrs 合并与无回声 | P0 | 全端 | ✅ | `mica-editor::crdt_two_peer_remote_merge_preserves_local_caret_and_undo` |
| `CAP-09-04` 远端合并选区重定位 | P0 | 全端 | ✅ | `mica-editor::reconcile_selection_tracks_remote_insert_and_block_shift` |
| `CAP-09-05` 本地 SQLite 离线存储与云同步重试 | P0 | 桌面/Web | 🟡 | `storage_and_sync`（17/17 通过），生产云端长连协作联调待专项实测 |

---

## 演进记录

- **2026-11-23**：沉淀单栈 CRDT 撤销、本地脏块保护与 `SelectionReconcile` 契约。
