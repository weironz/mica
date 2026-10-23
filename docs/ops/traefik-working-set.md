# Traefik 工作集保护（2026-10-23 最终配置）

本文记录 `mica.cloudcele.com` 共享入口已实际保留的修复。普通 Mica 发版不要重建它；
已有节点已经配置完成，本次文档更新也不触发生产部署。

## 最终配置与持久化位置

| 项目 | 最终值 | 仓库 / 生产位置 |
| --- | --- | --- |
| 内存硬上限 | `256m` = 268435456 bytes | `deploy/traefik/docker-compose.yml` / `/data/traefik/docker-compose.yaml` |
| 容器软保护 | `mem_reservation: 128m` = 134217728 bytes | 同上；对应 leaf `memory.low` |
| 独立父组 | `cgroup_parent: traefik.slice` | 同上；不要放进 `mica.slice` 或修改共享 `system.slice` |
| 根组软保护 | `MemoryLow=128M` = 134217728 bytes | `deploy/traefik/traefik.slice` / `/etc/systemd/system/traefik.slice` |
| RAM + swap 策略 | YAML 整数 `memswap_limit: -1` | 保留 Traefik 原有无限 swap 策略；没有关闭宿主机 swap |
| 强保护 | root / leaf `memory.min=0` | 没有使用 `MemoryMin` 或锁住 Traefik 全部内存 |

软保护是回收偏好，不是预分配，也不创造物理内存；资源竞争严重时不能保证绝不被回收。
root / leaf 是同一保护额度的两级配置，不能相加成 256 MiB 的额外保留内存。
在本机 cgroup v2 层级中，只配 leaf 而祖先保护为 0 不足以达到这次目标；根级独立 slice
和 leaf 都要核对。参见 [内核 memory 接口说明](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html#memory-interface-files)。

此次保留的镜像仍为 `traefik:v3.6.10`，实际镜像 ID 为
`sha256:71678227393a5f65b692d193e0e41ef6f505e9e4ed943b030ed1826eb6881983`。
镜像、完整容器 Config、挂载、端口、网络别名与本次修改前核对一致。
日志沿用本次修改前的 `json-file`、`max-size=50m` / `max-file=3`；没有修改日志参数、
Docker 全局设置或其他项目配置。日志不是本次资源模板的管理项，升级前记录实际值。

## 为什么要两层保护

现场观察到共享主机约 3.49 GiB RAM、约 4 GiB swap 已满，memory / I/O PSI 极高。
Traefik 在 D 状态等待缺页读盘：既有 `swap_readpage` 路径，也有
`filemap_fault → xfs_filemap_fault` 文件映射缺页路径。因此不能把所有等待都叫作 swap
换入，也不能将 swap 占用率当成某次请求的耗时。

- 只读定位曾捕获 Traefik TLS 约 1.995 秒；同窗口的包时间、major fault 与停顿证据
  将等待定位到 ClientHello 后的入口处理。`127.0.0.1:443` 另含 Docker 用户态代理，
  其耗时不能全算给 Traefik；直连容器 IP 必须保留正确的 SNI / Host。
- 分步试验先在线将硬上限从 128 提到 256 MiB，保持保护为 0、没有重建。
  暂停主动测试 222 秒后的 TLS 为 291 ms、完整请求 1045 ms；后续 Chrome 请求仍
  20 秒超时，服务器直连 Traefik TLS 也 10 秒超时，而直连 Mica API 约 1.7 ms。
  单提上限没有解决工作集被回收的问题。
- 用户随后批准增加独立 128 MiB 保护并仅重建 Traefik。最终样本中 Traefik swap 为 0、
  memory PSI 接近 0；宿主机 PSI some 仍约 98–99%，Mica 公网验收却恢复，支持入口
  工作集保护有效，不代表整机资源压力已消除。

搜索优化与 swap 参数的历史应分开：

| 提交 | 改动与边界 |
| --- | --- |
| `66a5477`，2026-10-21 | 新增 API `IPC_LOCK` 和默认开启的 `MICA_SEARCH_INDEX_MLOCK`；未新增 `memswap_limit`。提交说明已记载此前约 2 GB swap 与换页。最终 API `VmLck` 约 80.8 MiB，未通过关闭锁定的对照实验确认其增量影响。 |
| `ced07ef`，2026-10-23 | 本轮此前新增 Mica 的 `mica.slice` / reservations，以及 PG、RustFS 的固定 `memswap_limit: -1`。改前未采集原 `MemorySwap`，不能称策略未变。 |

最终 Mica 整组 swap 约 8.2 MiB（PG 约 1.1、RustFS 约 6.7 MiB），解释不了宿主机约
4 GiB 的全部占用。现有证据不足以认定搜索优化或新 `-1` 参数独自造成整机压力。

## 以后部署如何保留修复

1. **普通版本发布**：`.github/workflows/deploy.yml` → `scripts/deploy-prod.sh` →
   `ansible/deploy.yml` 只部署 Mica。应用 Compose 来自应用 tag；这条路径不复制
   `/data/traefik` 文件、不重启 Traefik、不停用其 slice。不要把入口混进普通发版。
2. **新节点 / 容灾重建**：使用包含本次资源修复或更新的受审阅仓库 checkout。
   `ansible/provision.yml` 先检查 systemd + cgroup v2，安装并启用
   `deploy/traefik/traefik.slice`，再安装包含四个资源字段的 Traefik Compose。
   入口基础设施来源不必跟随待恢复的旧应用 tag；旧模板会丢掉保护。
3. **Compose 文件名**：仓库源为 `.yml`，生产与 provisioning 目标统一为
   `/data/traefik/docker-compose.yaml`。Ansible 显式指定 `files: [docker-compose.yaml]`，
   人工命令也必须 `-f docker-compose.yaml`；不要依赖 `.yml` / `.yaml` 并存时的自动选择。
4. **已有共享节点维护**：先备份现有文件和运行配置，只向当前入口 Compose 合并上表
   四个资源字段，保留真实环境变量、路由、证书卷、镜像和日志。不要整份覆盖自定义配置，
   也不要为此运行完整 `provision.yml`——该 playbook 是新节点安装流程，还会管理 Docker。
5. **变更验收**：`scripts/test-compose-api-env.py` 会解析真实 Compose，核对硬上限、
   leaf reservation、swap、父组与 slice 根保护匹配；本地和现有 CI 配置检查均运行它。
   该回归检查不连接 Docker daemon，也不启动或更新任何服务。

已应用的这次运行状态与 Compose / systemd 均持久化；没有做主机重启验证。
仅 `docker update` 不足以持久化到下一次容器重建；只改 slice 文件也要 reload / 启用并
检查实际层级。改变父 cgroup 需要重建，会影响共享入口上的其他业务，应安排维护窗口。

### 仅供获批的入口维护：备份、安装、应用

以下为将来维护步骤，**不是普通 Mica 发版步骤**。保持证书卷和网络，不执行 `down -v`，
也不重启 Docker。备份含环境变量和完整 inspect，留在 root-only 目录，不提交仓库。

```sh
cd /data/traefik
umask 077
backup_dir="/data/traefik/ops/pre-working-set-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -m 700 "$backup_dir"
cp -p docker-compose.yaml "$backup_dir/"
for file in .env .env.secrets; do
  if [ -f "$file" ]; then cp -p "$file" "$backup_dir/"; fi
done
docker inspect traefik-traefik-1 > "$backup_dir/inspect.private.json"
if [ -f /etc/systemd/system/traefik.slice ]; then
  cp -p /etc/systemd/system/traefik.slice "$backup_dir/"
fi
```

从受审阅的 Mica 仓库目录安装 unit：

```sh
sudo install -m 644 deploy/traefik/traefik.slice /etc/systemd/system/traefik.slice
```

仅向已有入口 Compose 合并上述四个资源字段后：

```sh
cd /data/traefik
sudo systemctl daemon-reload
sudo systemctl enable --now traefik.slice
docker compose -p traefik -f docker-compose.yaml config --quiet
docker compose -p traefik -f docker-compose.yaml up -d --no-deps --pull never traefik
```

检查镜像、日志与非资源配置差异。`memory.max` 应为 268435456，root / leaf
`memory.low` 均为 134217728，`memory.min` 均为 0，`system.slice` 不应被调整：

```sh
docker inspect --format '{{.State.Status}} {{.State.Health.Status}} {{.HostConfig.Memory}} {{.HostConfig.MemoryReservation}} {{.HostConfig.MemorySwap}} {{.HostConfig.CgroupParent}}' traefik-traefik-1
pid=$(docker inspect --format '{{.State.Pid}}' traefik-traefik-1)
cg=$(awk -F: '$1 == "0" {print $3}' "/proc/$pid/cgroup")
cat "/sys/fs/cgroup$cg/memory.max" "/sys/fs/cgroup$cg/memory.low" "/sys/fs/cgroup$cg/memory.min"
cat /sys/fs/cgroup/traefik.slice/memory.low /sys/fs/cgroup/traefik.slice/memory.min
systemctl is-enabled traefik.slice
```

容器 healthy 或 TLS 成功不等于业务路由已加载；还要确认公网 `/api/health` 返回 200、
新建测试文件夹与目录切换成功、空闲后新连接成功，并分层观察其他项目。
若异常，按当次授权定位修复或撤回；不要直接运行旧试验目录内的 `rollback.py`。

## 最终验收结果及影响

全部为 2026-10-23、Asia/Shanghai 的历史实测，不作为未来性能保证。

| 操作 | 最终实测 |
| --- | --- |
| 停止主动 Mica 探测 233 秒，独立 curl 新连接 | TLS 32 ms；HTTP 200；完整请求 63 ms |
| 再停止 222 秒，新 Chrome HTTP/2 | TLS 29.9 ms；`connectionReused=false`；HTTP 200；总计 74 ms |
| 浏览器新建测试子文件夹 | HTTP 200，191 ms；命名成功、确认可见 |
| 小目录 / 3080 页目录 / 返回原目录 | 378 / 499 / 543 ms，均 HTTP 200；返回后新节点可见 |
| Traefik 最终状态 | healthy、重启计数 0、swap 0；没有新增触顶或 OOM kill |

只控制了本次主动 Mica 流量，其他用户和共享入口流量未停止；不能称整机完全空闲。
浏览器复用 PAT 恢复会话，业务 REST 真实到生产，写入只限专用测试根；没有覆盖完整
Cookie 登录或 WebSocket 协作。切换计时截至目录响应与两帧 UI 更新，不含全部文档
后台 bootstrap。Windows 此前由用户手动确认新建与切换约 1 秒内，未重新自动计时。

此次重建**确有共享入口中断**：15:12:08 开始切换，容器启动后路由曾数分钟返回 404，
15:20:23 前已核对业务恢复；没有精确测得连续不可用时长。同期 Docker 发现列表读取
超过 25 秒，单个容器信息读取 8–12 秒，与路由加载延迟同时出现，不能只报 TLS 快样本。

两段观察（约 10 分钟、约 5 分钟，中间有间隔）及抽查中，另外 48 个运行中容器未发现
新增停止、重启或 OOM kill，但宿主机压力与部分健康探针波动仍存在。15:40:39 对照中：
NeoStor 经入口 136 ms、直连后端 42 ms；NetBox 经入口 4030 ms（TLS 64 ms），直连
后端也 4008 ms，后者剩余延迟在后端。未修改这些项目配置，不能称全部业务都验收通过。

最终没有自动回退，没有扩容。本轮实施前完整备份与私密收据保存在：

- 在线 cap 试验：`/data/traefik/ops/cap256-20261023T064931Z`。
- 最终独立保护：`/data/traefik/ops/protect128-20261023T071027Z`。

仅保留文件备份，已删除停止的旧 Traefik 容器，未删除卷。私密 inspect / payload /
engine 日志不公开；此文只保留脱敏结果。此前更早的一次 256/128 试验曾撤回，不能
将其与这里**重新批准、最终保留**的配置混为一谈。
