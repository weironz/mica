# 编辑器 P2：协作撤销与资源缓存修复设计

对应 [#79](https://github.com/weironz/mica/issues/79)、[#62](https://github.com/weironz/mica/issues/62)。先设计，后实现；不提交 git commit。

## #79：撤销本地操作

### 源码证据与取舍

当前 `EditorController` 每次 `_send` 把整篇 `_DocSnapshot` 入栈；远端修改过一个块后，该块整步跳过。这既无法撤销同段中的本地输入，也把历史内存放大成文档大小 × 步数。

先核对了同类产品的真实源码：

- [AppFlowy-Collab](https://github.com/AppFlowy-IO/AppFlowy-Collab/blob/be5aa89b4aeafd4e7159e92b86784c02caaa85ce/collab/src/core/collab.rs#L607)：yrs `UndoManager` 限定文档 data 范围，`include_origin(self.origin())`；本地事务带相同 origin，远端合并不进入个人历史。
- [AFFiNE / BlockSuite](https://github.com/toeverything/AFFiNE/blob/fbf2cebfa012a7c92b08af7bfa0340177308b2a6/blocksuite/framework/store/src/extension/history/history-extension.ts#L22)：`Y.UndoManager([doc.yBlocks], trackedOrigins={clientID})`；[本地事务](https://github.com/toeverything/AFFiNE/blob/fbf2cebfa012a7c92b08af7bfa0340177308b2a6/blocksuite/framework/store/src/model/store/store.ts#L371) 带 clientID，`captureSync()` 显式切断分组；[选择区](https://github.com/toeverything/AFFiNE/blob/fbf2cebfa012a7c92b08af7bfa0340177308b2a6/blocksuite/framework/store/src/extension/selection/selection-extension.ts#L22) 是历史项的元数据。
- [Yjs 的协作撤销测试](https://github.com/yjs/yjs/blob/89dddc2a95079460b7203bde07f45dffd56629f2/tests/undo-redo.tests.js#L82)：本地 abc + 远端 xyz，撤销只去 abc；远端已删的本地字符不会被 redo 复活。[远端覆盖 map 属性](https://github.com/yjs/yjs/blob/89dddc2a95079460b7203bde07f45dffd56629f2/tests/undo-redo.tests.js#L163) 也保留远端值。

采用现有 yrs / yjs 的本地事务逆操作历史。不能从渲染层的纯文本快照可靠推回字符身份（相同字符尤其含糊），所以正式本地/云端编辑器通过后端历史接口直接撤销 CRDT 操作，再读取当前块；不把旧全文 diff 写回 CRDT。

### 接口与不变量

1. 核心 `MicaDoc` 按需启用 history，scope 为 blocks；只有显式 editor batch 的事务带 editor origin。加载、远端 `apply_update`、标题/修复等写入不捕获。每个现有提交批次是一个撤销组，保持 400ms typing burst 与 split/merge/move/AI 等复合命令的既有分组。
2. 原生桥与 Web yjs 提供 begin/end batch、reset、undo/redo、canUndo/canRedo 和 historyDepths。两端都只跟踪 editor origin，undo/redo 本身不重新记成普通编辑。一次 undo 可能跳过已被远端覆盖的历史项，选择区元数据按引擎返回的栈深度裁剪，不能只弹一项。
3. `LocalDocBackend` / `CloudSyncSession` 把普通编辑和撤销生成的 CRDT diff 送入原有 append-log / outbox；撤销和持久化的错误仍通过原有错误通道报告。远端操作不清 redo。
4. 编辑器新增可选历史回调；正式外壳绑定本地/云端对应实例。撤销先 flush，串行等待已有发送完成，再调用后端；结果更新 live nodes。历史元数据只保存操作逆向补丁与按块 id 的选择区，不保存每步全文。
5. 独立嵌入、测试使用普通 `onOps` 时仍有本地逆操作日志：只存本地批次触及的字段、增删块内容、相邻块锚点。没有 CRDT 引擎的自定义 sink 遇到远端覆盖时按字段保守跳过；正式协作路径必须使用引擎历史。
6. 新逻辑放独立模块/part；不增加 editor.dart/main_shell.dart/main_workspace_view.dart 的棘轮行数。

历史属于当前编辑器会话，打开/重建编辑器时 reset，不持久化历史栈。独立 block-op sink 仍最多 200 步；yrs 0.28 没有安全的栈裁剪 API，正式引擎路径保留会话中的 CRDT 历史，关闭会话后释放，不伪造 200 步截断。选择区按块 id 恢复，并将数值 offset 限制在当前文本长度内；这里没有宣称数值 offset 具有 CRDT relative-position 的完整语义。

### 验收

Rust 与 Web 真引擎分别验证：同段本地/远端交错输入；重复字符、UTF-16；远端删掉本地字符；本地 undo 后远端编辑再 redo；属性被远端覆盖；split/merge/insert/delete/move；历史生成的更新在另一副本收敛。Flutter 验证提交顺序、选择区按块恢复、默认逆操作日志和已有编辑行为。检查核心 crate、FFI crate、Web bundle 与 Flutter analyzer。

## #62：资源缓存收敛

问题链已经由 issue 和源码定位：全文布局请求资源，容量驱逐仍在文档中的资源，结果通知清布局，全文再次请求。图片已有 paint-touch LRU，但全文请求仍形成相同反馈环。

实施前先用超过 48 个公式、64 张图片的真实编辑器布局/帧测试证明请求持续增长，再据实确定视口需求集合。缓存策略必须同时满足：只请求视口附近资源；paint touch LRU；一个视口本身超过容量时仍能稳定（当前需求保活，软容量仅驱逐非当前需求）；几何信息不因位图驱逐反复退回占位；图片释放遵守帧边界，异步旧结果不能重新污染主题/文档的新缓存。

详细实现和实测数据在处理 #62 时补充。
