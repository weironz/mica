# 跨 workspace 移动 / 复制页面·目录(#3)

把一个页面(及其子树)或文件夹从一个云端工作区搬到**同一服务器上的另一个云端
工作区**。菜单出「移动到工作区…」「复制到工作区…」。本地↔云是单独的 #4。

## 结论:必须是「复制到目标 + 软删源」,不能原地 re-parent

三方源码调研(2026-07)——**没有一个 CRDT 笔记应用做真·跨工作区原地移动**:

| 产品 | 跨工作区移动 | 实现 | 代价 |
|---|---|---|---|
| Notion | 有 "Move to" | **复制到目标 + 原件保留**(官方文档明说 "duplicated") | 新 page id;历史/评论/权限/关系可能断;文件重新托管;非原子("keep original until sure") |
| AFFiNE(Yjs) | **无原地移动** | 只能 Export→Import Snapshot(复制、新 doc-id),后来还被删 | doc 身份按 `workspace/doc_id` 命名空间,无法 re-parent |
| AppFlowy(yrs) | **无** | "Move to" 只在同工作区内 | 官方"有带宽再做";维护者明确点出跨工作区引用会成死链 |

**根因(和 Mica 同构)**:doc 身份 + blob 都按工作区命名空间。AFFiNE `doc_blob_refs.rs`
里 blob 引用按 `(workspace_id, doc_id)` 键、缺席 doc 的引用会被 `purge_removed_doc_refs`
清掉——这正是 Mica 的 blob GC。**原地 re-parent 会招 doc-id 冲突、客户端缓存 split-brain、
源工作区 GC 把还被引用的图回收**。所以业界一律 copy-into-dest。

**Mica 的杀手锏**:Postgres 事务。Notion/AFFiNE 都非原子(丢文件、"留着原件保险")。
Mica 把「blob 落目标 → 建目标树 → 删源」做成原子 + blob 先行,半路失败只在目标留下
无害孤儿字节(目标 GC 回收),**绝不丢图**。这是我们能赢的一点。

## Mica 数据面(为什么这么设计)

- `documents`/`views` 有 `workspace_id`;`document_updates`/`snapshots`/`versions` 按
  **document_id** 键。→ 新建 doc 会拿到新 doc-id,旧版本历史/快照**不跟随**(按旧 doc_id)。
- `files.object_key = workspaces/{ws}/{sha256}.{ext}`,**内嵌 workspace_id**。blob 按工作区拥有。
- blob GC(`blob_gc.rs`):按工作区扫 `views`(含回收站)算引用集,`files WHERE workspace_id=$1`
  里没被引用的 → 标记 → **30 天宽限 + 7 天最小年龄**后删对象 + 行。→ 文档移走后源不再引用
  它的 blob,30 天后源 GC 回收 → **不复制 blob 就会图裂**(红线)。

## 已批准的决策(2026-07-17)

1. **移动 + 复制都做**,移动为主。复制=留源(安全兜底);移动=复制成功后软删源。同一套机制。
2. **版本历史检查点不跟随**(新 doc-id),移动前弹框如实告知。同类产品全这样。
3. **指向"留在源"的页面内部链接** → 检测出来 + 弹框警告 + **保留原样**(不自动把被链页拽过来)。
4. **v1 仅云端同服务器互移**。跨服务器/本地留给 #4。

## 数据操作(顺序是红线)

前置:调用者需**源 + 目标都有 editor 权限**;dest≠src;若给 parent_view_id 必须在 dest 且是 folder。

1. **枚举子树**:`WITH RECURSIVE subtree` 从 view_id 起(含自身),取全 View 行,pre-order。
2. **预扫**:每个 document view 读 `current_payload`,收集引用的 `file_id`;检测页面链接
   (`mica://page/<viewId>`)目标**不在子树内**的 → 跨工作区死链列表(供警告)。dry_run 到此返回。
3. **复制 blob(事务外,幂等)**:每个唯一 file_id → 取源 `files` 行 → 从源 `storage.download_url`
   GET 字节 → dest object_key = 把 `workspaces/{src}/` 换成 `workspaces/{dest}/` → `presign_put` +
   PUT。(复用 `import_url` 的 GET→PUT 模式。)记 (dest_object_key, name, mime, size)。
4. **事务**:
   - 建 src_view_id → dest_view_id 映射(全子树生成新 uuid)。
   - `insert_file`(ON CONFLICT(object_key) DO UPDATE RETURNING,幂等去重)→ 拿 dest file_id,
     建 src_file_id → dest_file_id 映射。
   - pre-order 遍历子树:
     - document:重写 payload —— `block.data["file_id"]` 按 blob 映射换;页面链接
       `mica://page/<srcViewId>` 对**子树内**目标换成 dest view id,子树外保留原样。然后 `insert_page`
       式:建 `documents` 行(新 doc-id)+ `insert_root_snapshot`(重写后 payload)+ 建 `views`
       行(新 view id,dest 工作区,dest parent=映射或根 view 的 parent_view_id 入参)。
     - folder:只建 view(object_type='folder',dummy object_id,新 id,dest parent)。
   - remove_source(移动):软删源子树(`is_deleted=true` 递归 CTE,进回收站——源 blob 宽限期内仍被引用,安全)。
5. 提交。返回新根 view + 报告(docs/folders/images 数、跨工作区死链)。

**复用原语**:`insert_page`/`insert_root_snapshot`(import.rs/store.rs)、`insert_file`(幂等)、
`current_payload`、递归 CTE 子树(documents.rs delete_view)、`rewrite_page_links` 模式、
`import_url` 的服务端 GET→PUT blob 复制。

## 里程碑

- **M1 服务端核心**:`crates/api-server/src/routes/transfer.rs` —— handler + blob 复制 + payload 重写
  (file_id/链接)+ 事务建目标树 + 软删源;路由
  `POST /api/workspaces/{src}/views/{view_id}/transfer` body `{dest_workspace_id, parent_view_id?,
  remove_source, dry_run}`;返回报告。Rust 测:复制保结构+blob+file_id 引用(round-trip)、移动软删源、
  死链检测、blob 落到目标工作区、原子性(半失败不丢图)。
- **M2 客户端 UI**:页面/文件夹菜单「移动到工作区…」「复制到工作区…」(云端才有);对话框选目标工作区
  (排除当前)+ 可选父节点 + 预览(N 文档/目录/图片 + 死链警告 + "版本历史不跟随"提示)+ 确认;
  API client;WS live-sync 反映源子树软删 + 目标新树。
- **M3 MCP**:`mica_move_document` 现仅同工作区;新增 `mica_transfer_view`(dest_workspace_id + copy 标志)。
- **M4 对抗复审 + 测试 + 发版**。

## 边界 / 已知取舍

- v1 不做:跨服务器、本地工作区(#4)、自动拽被链页、保留版本历史。
- 死链:子树内页面互链会 remap;指向源的链接保留 `mica://` 原样(会断,已警告)。
- 图片共享:同一 blob 被多文档引用时,copy-into-dest 天然安全(目标建自己的副本,源副本留给源 GC)。
- 移动 = 软删源(进回收站),不是硬删——用户可从回收站恢复;源 blob 宽限期内不被回收。
