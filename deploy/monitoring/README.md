# 监控示例栈(可选)

Mica 的部署里**没有**抓取器。`/metrics` 是应用的一部分,谁来抓、留多久、谁能看,是运维的决定,
不是 Mica 的部署内容 —— 分开的直接好处是:监控栈升级、挂掉、写满磁盘,都带不走应用;而已经有
Prometheus 的人可以直接指过来,整个目录当不存在。

这里是一份能直接跑起来的示例:Prometheus + Grafana。

## 起

```bash
docker compose -f deploy/monitoring/docker-compose.yml up -d
```

它以**外部网络**方式接进 Mica 的 compose 网络 —— 那是通往 `/metrics` 的唯一路径(nginx 不代理它)。
默认接生产栈的 `mica_default`;接本地 dev 栈要改:

```bash
MICA_NETWORK=$(basename "$PWD")_default docker compose -f deploy/monitoring/docker-compose.yml up -d
```

网络名对不对,`docker network ls` 一看便知。

## 看

两个 UI 都只绑 **loopback**:指标界面挂在公网等于免费送一张系统地图,而 Grafana 的默认口令是公开
知识。走 SSH:

```bash
ssh -N -L 9090:127.0.0.1:9090 -L 3000:127.0.0.1:3000 root@<host>
```

- Prometheus <http://127.0.0.1:9090> —— `Status -> Targets` 应看到 `mica-api` 是 `UP`
- Grafana <http://127.0.0.1:3000> —— 默认 `admin` / `admin`(`GRAFANA_ADMIN_PASSWORD` 可改),
  数据源已通过 provisioning 配好,直接进 Explore

## 几条起手的查询

```promql
# 请求速率,按路由
sum by (route) (rate(mica_http_requests_total[5m]))

# 5xx 占比
sum(rate(mica_http_requests_total{status=~"5.."}[5m]))
  / sum(rate(mica_http_requests_total[5m]))

# p95 延迟
histogram_quantile(0.95, sum by (le) (rate(mica_http_request_duration_seconds_bucket[5m])))

# 连接池压力 —— 2026-08-03 那次事故里,存活探针一切正常,是它先动的
mica_db_pool_connections

# 每个工作区的用量占配额比例(UUID 靠 info 指标 join 出可读名字)
mica_workspace_bytes_used * on(workspace_id) group_left(name) mica_workspace_info
  / ignoring(name) mica_workspace_quota_bytes
```

## 没做告警

有意的。告警规则得先有人接、有人值、有下班时间的约定,否则只是把噪音写进配置文件。指标先攒着,
等真需要时按当时的痛点写规则,比现在凭空猜阈值准。

指标清单见 `crates/api-server/src/metrics.rs`。
