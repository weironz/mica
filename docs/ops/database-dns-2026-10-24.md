# 2026-10-24 Mica 数据库 DNS 故障

## 现象与证据

桌面端切换工作区、将 `just` 从 rust 移到 devops 后出现 `internal database error`。
同期独立 REST/MCP 搜索 rust、devops 也返回 HTTP 500，排除仅桌面 UI 的解释。
API 在 02:03–02:10 UTC 多次记录数据库连接错误：

```text
failed to lookup address information: Temporary failure in name resolution
```

API 的数据库主机为 Docker 服务别名 `postgres`。从 API 网络命名空间直连实际
PostgreSQL 地址的 TCP 探测成功（约 158 ms）；数据库日志未见对应 SQL/约束错误。
故障直接发生在连接建立时的主机名解析，共享宿主机当时负载很高，但尚未证明
Docker DNS 内部哪一步阻塞，不能将其进一步断言为某个 swap 参数的单一因果。

`just` 及其对应文档已在 devops 中，文档仍非空（seq 7）；rust 中已无同名页面。移动后的
刷新失败与移动事务失败是两回事，没有再次移动或改写该用户页面。

## 修复及边界

- 02:21:26 UTC，备份后仅向当时 Mica API 的 hosts 加入数据库实际地址映射，
  没有重启。之后两工作区只读查询恢复。
- 备份在节点 `/data/mica/ops/db-dns-20261024T022126Z`，目录权限 0700。
  内含原 hosts、私有容器配置及操作收据；配置含凭据，不进入仓库或公开日志。
- 两份 Compose 支持 `MICA_DATABASE_HOST`，默认继续使用 `postgres`。生产
  host_vars 指定核实的 Mica PostgreSQL 私网地址，普通发布据此生成 API 连接串。
- Ansible 在写部署文件之前检查运行中的 `mica-postgres-1` 实际地址。陈旧地址、
  停止的数据库、空地址都拒绝部署。计划重建数据库后必须重新核实并更新地址。
- 连接 I/O/连接池不可用返回脱敏 HTTP 503，SQL/迁移错误仍是 500。没有自动重试
  写操作，避免连接中断后将已提交的移动或写入重复执行。
- 本次不修改 PostgreSQL 配置、Docker 全局 DNS、共享 Traefik 或其他项目。
  普通发布继续使用 deploy.yml，不运行 provision.yml。

## 源码验证

- Compose 测试覆盖两种部署的默认别名、显式地址覆盖及凭据/端口/库名保持不变。
- Ansible 本地用例直接执行真实预检任务：正确地址通过；陈旧、停止、空地址拒绝。
- infra 测试覆盖连接故障的 503、错误详情脱敏及真正查询错误保留 500。
- main 库将父子排序移到独立 library，从 21,371 降至 21,349 行；原预算 21,355
  收紧为 21,349。迁移排序与工作区迁移相关 16 项测试通过。

发布后的版本、功能和运行日志验收另补实测结果，不把上述源码测试当作生产验收。
