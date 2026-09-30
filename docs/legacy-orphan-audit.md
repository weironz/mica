# 历史孤儿对象盘点

`pending_file_uploads` 只能追踪部署台账以后签发的上传和服务端写入。旧版本中 PUT 成功、`complete` 未完成的对象没有数据库行，常规 GC 找不到它们。新版 API 提供一次性盘点命令；**它不会直接删除对象**。

依赖豁免：S3 的 `ListObjectsV2` 返回 XML，本命令在平台协议边界使用已在 `Cargo.lock` 中的 `roxmltree` 解析。它不进入 CRDT、文档模型或同步数据面；手写 XML 解析器在实体与转义处理上更容易误判对象键。

在部署目录执行，使用当前 API 容器的数据库和对象存储配置。生产节点必须用 `./dc` 加载 `.env.secrets`，不要直接调用 `docker compose`：

```sh
./dc exec -T api mica-api-server audit-legacy-orphans > legacy-orphans.jsonl
```

默认只读。逐行检查 `kind=legacy_orphan` 的键，重点核对对应工作区以及是否有人工保留的对象；`kind=unexpected_key` 只报告，永远不会回填。末行 `kind=summary` 给出候选数。工具只列举 `workspaces/` 前缀，`avatars/` 不在范围内。

确认候选对象可以交由延迟回收后，执行：

```sh
./dc exec -T api mica-api-server audit-legacy-orphans --backfill > legacy-orphans-backfill.jsonl
```

该命令重新核对 `files` 与台账，给未跟踪的合法内容哈希键建立台账；重复运行安全。现有 GC 至少再等待 30 天，并复核对象的实际修改时间，才可能删除仍无文件记录的对象。对象存储分页、响应格式或数据库出错会让命令失败退出；检查输出的 `kind=summary` 确认完整运行，不要把部分输出当成全桶结果。生产盘点只在新版部署后执行。
