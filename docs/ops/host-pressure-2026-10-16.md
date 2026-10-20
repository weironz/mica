# 2026-10-16 生产主机 Swap / I/O 定位

对象：`mica.cloudcele.com`。时间均为 Asia/Shanghai；本轮接续 v0.13.52 备份验收，
重点核对备份结束后的整机压力。初始诊断为只读；用户随后批准两项生产优化。
Vault watcher 已应用；用户随后纠正项目范围，Datazone/UFM 操作已停止，未应用。
以下初始采样和后续变更按时间分别记录。

## 采样边界

主机为 2 vCPU、3,657,008 KiB RAM（约 3.49 GiB）、4 GiB Swap，共 47 个运行容器。
Swap 文件位于系统盘；应用数据和 Docker 存储位于 `/data` 数据盘。
数据盘 100 GiB，已用约 95%，可用约 5.2 GiB；这是容量余量问题，不能仅据此认定磁盘带宽打满。

10:19:33 起先做约 123 秒进程、cgroup、主机同步采样。该窗口包含其他诊断的冷启动探针，
例如读取 scanner 状态、metrics 和容器内静态文件，故不能作为完全无人访问的后台基准。
该窗口 Swap 换入约 69.1 MiB、换出约 215.1 MiB；它反映当时压力，不能全部归因于自然后台任务。

随后停止全部服务 HTTP / metrics / docker exec 探针，10:28:39 起只读 `/proc` 和 cgroup 90 秒。
每 10 秒统计差分，丢弃前 20 秒，剩余窗口为 **10:28:59.786–10:30:11.501，71.714 秒**。
采样器本身仍有约 1–2% 单核 CPU 开销，真实外部业务和后台采集并未暂停。
CPU 的 100% 代表单核，2 vCPU 主机总容量是 200%。I/O 是窗口差分，内存为窗口末 cgroup 值，
包含匿名页、文件页及内核计费，不能与进程 RSS 或 Swap 直接相加作为峰值。

| 服务 | CPU | 物理读 MiB/s | 写 MiB/s | 当前内存 MiB | Swap MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| NeoStor VictoriaMetrics | 59.79% | 2.050 | 0.232 | 496.69 | 48.24 |
| NeoStor server | 20.13% | 0.142 | 0.000 | 86.68 | 21.64 |
| NeoStor Vault unseal watcher | 1.64% | 0.599 | 0.000 | 169.11 | 0.02 |
| NeoStor Vault | 1.27% | 0.398 | 0.000 | 151.36 | 10.30 |
| NeoStor Grafana | 0.16% | 0.261 | 0.001 | 88.62 | 13.45 |
| Infisical | 2.36% | 0.082 | 0.027 | 549.92 | 174.21 |
| NetBox web | 0.50% | 0.143 | 0.001 | 87.76 | 531.88 |
| NetBox worker | 0.71% | 0.011 | 0.003 | 11.98 | 240.35 |
| Mica RustFS | 0.11% | 0.051 | 0.005 | 130.44 | 27.53 |
| Mica API | 0.59% | 0.018 | 0.016 | 24.42 | 167.45 |
| Mica backup | 0.00% | 0.000 | 0.000 | 2.66 | 0.67 |

静默窗口总 Swap 换入 **19.750 MiB**、换出 **11.768 MiB**，平均约 0.275/0.164 MiB/s。
窗口末 Swap 已用约 2,029 MiB，MemAvailable 约 1,220 MiB。存量 Swap 不等于持续换页速率。
所有容器在该窗口内 memory.events 的 max / OOM / OOM-kill 增量均为零。
末次主机 PSI avg10：memory some/full 为 3.01%/1.18%，I/O 为 1.48%/0.16%，CPU some 为 47.43%。
这些是采样末十秒指数均值，不是整段累计百分比。

后续独立 iostat 短窗中，vda/vdb 读请求等待约 0.75–3.55 ms，未观察到持续队列饱和；
vmstat I/O wait 为 0–1%。短窗不能覆盖之前所有峰值，也不能证明磁盘性能永久正常。

## 已定位的后台工作

### 1. VictoriaMetrics 持续接收高基数遥测

实际为 v1.103.0，512 MiB 内存上限，保留期 6 个月。静默窗口持续占单核约 60%，
末次 cgroup 内存约 497 MiB。现有证据支持它是当前主要 CPU 和读写进程。
10:19:10–10:21:31 的写入计数增加 606,763 行，约 **4.30k samples/s**；
不能把基线错放到 10:20:00 而算成 6.7k/s。

五组 SNMP 传感器指标的 TSDB 日期统计合计约 278,393 条时间序列；这是日期范围统计，
不能当作当前活跃序列数。`entPhySensorValue/Type/Precision/OperStatus/Scale` 在 NeoStor
设备详情中用于解释实际数值，不能为降低基数直接删除。

Grafana 只有一个八面板看板、没有 Grafana alert rules，数据源请求计数在采样中没有增长。
中心 vmalert 为六条规则、60 秒求值，未见漏算或查询并发打满。没有发现看板轮询风暴。
NeoStor server 承担每批遥测认证、解压、校验和转发；其约 20% CPU 尚无独立 profiler 证据，
不能把全部 CPU 都归为某一个 handler。

10:36:36 对 Datazone 的一次窄范围只读核对确认，独立的
`ufm-neostor-dz-chonburi-ufm-forwarder-1`（vmagent v1.103.0）单出口指向本机
`127.0.0.1:19091/api/v1/write`，进入 NeoStor。它与双出口 shared-vmagent 是不同容器。
`ufm-telemetry` 抓取 UFM `:9628`，**180 秒**一次、190,028 samples/次，最近抓取 up，耗时 0.416 秒；
`ufm-rest` 抓取 `:9627`，60 秒一次、383 samples/次。
不能按 60 秒估算 telemetry，也不能把历史 TSDB 基数当成每次样本数。

正式 NeoStor 前后端、六条告警及现有八个 Grafana 面板未找到这批 UFM 原始 IB 指标的消费者。
最小候选为仅取消专用转发器的 `ufm-telemetry` job，保留 `ufm-rest`、共享 SNMP/Redfish、
原有 Prometheus 全量采集和 UFM exporter。按已核实的周期，可减少约 **1,055.7 samples/s**，
相当于上述中心总写入约 24.5%。这是样本数估计，不能推出 CPU、内存、I/O 同比例降低。
应用时保留历史数据，先备份本地私有配置，验证配置并使该专用进程 reload，随后跨至少两个
180 秒周期验证目标、中心增量和正式监控路径。此候选尚未实施。

### 2. Vault watcher 每 15 秒启动完整 CLI

线上 watcher 的 shell 循环每 15 秒执行一次 `vault status`，已封印才执行自动解封。
静默窗口它仍有 0.599 MiB/s 物理读，末次匿名页仅约 0.10 MiB、文件页约 168 MiB。
这个行为是可直接消除的周期性完整程序启动；可以使用现有 BusyBox wget 请求轻量健康接口，
仅在明确 sealed 时调用解封 CLI，保留原有 15 秒检查和密钥挂载。

Vault 服务与 CLI 使用的可执行文件页可能共享，不能承诺删除轮询后立即释放全部 168 MiB，
也不能在应用前宣称降低 I/O 的比例。需要部署后用同样静默窗口对照。

### 3. 其他服务的读盘包含被回收文件页重新读入

RustFS rc.3 scanner 状态为 inactive；最近一次 usage 扫描仅 1.96 秒、35 个对象，
heal / bitrot / 生命周期 / 复制任务均为零，idle backoff 已达到约 42 分钟。
没有持续扫描、打开对象文件或大量日志写入的证据。

部分 RustFS 线程累计大量物理读，但 read syscall 字节极少；ELF 约 221.8 MiB，
可执行文件映射实际驻留约 10.8 MiB，历史 workingset_refault_file 很高。
Grafana 也有类似代码/文件页 refault 证据，查询和 SQLite 日志均没有显示数据库扫描风暴。
这支持文件页被回收后重新读取的方向；它不等于所有物理读都是 Swap，也不能精确还原旧峰的调用栈。
静默期 Mica RustFS 和 Grafana 读盘均明显低于诊断探针活动期间，故不调整 scanner 或盲目扩大上限。

### 4. NetBox 存量 Swap 高，Infisical 有正常 cron 和限制漂移

NetBox Web 已是 Granian 单 worker，另有一个 RQ worker；不存在可直接减少的多 Web worker。
RQ fork 的父子 VmSwap 有共享/COW，按容器或 SwapPss 统计，不能把两者各约 240 MiB 简单相加。
静默窗口其换页相关活动很低，没有依据停掉业务 worker。

Infisical 为 v0.166.3，单 Node，old-space 512 MiB，数据库池 0–5，telemetry/update check 已关闭。
30 分钟内观察到 56 次 cron 开始、56 次完成，涉及审计、PAM、证书、扫描回收等；没有任务重试风暴。
改成 api-only 会停止真实职责；进一步压低 heap 尚无 GC/heap 证据支撑。

发现实际 Swap 约束漂移：Compose、Docker HostConfig 和 OCI 均为 memory=memorySwap=768 MiB，
目标为无额外 Swap，但 systemd scope 的 MemorySwapMax=infinity，内核 memory.swap.max=max。
Docker 支持 SwapLimit、无资源 warning，PID/cgroup 身份一致，祖先也没有 Swap 上限。
运行时 runc 1.3.4 依赖 cgroups 0.0.4，该版本 systemd 转换遗漏显式零值；容器启动后
10 月 15 日还发生过 PID 1 reexecute/reload，符合上游已知缺陷触发条件。
缺少当时 cgroup 值前后记录，故为高度吻合而非直接观察到那次重置。

上游证据：[cgroups Issue 74](https://github.com/opencontainers/cgroups/issues/74)、
[修复 PR 78](https://github.com/opencontainers/cgroups/pull/78)、
[runc 1.3.4 依赖](https://github.com/opencontainers/runc/blob/v1.3.4/go.mod)、
[零值遗漏代码](https://github.com/opencontainers/cgroups/blob/v0.0.4/systemd/v2.go#L216-L223)。
检查时修复 PR 尚未合并。不能承诺升级现有运行时就能解决，或一次 docker update 即永久修复。
当前也不直接禁用已有 Swap：可能把可恢复的分页变成 OOM。

## 当前结论和变更边界

v0.13.52 的备份在本窗口休眠，不是当前持续压力源。47 个容器共用 3.49 GiB RAM，
已有存量 Swap 和文件页回收竞争，当前持续 CPU/存储工作以 NeoStor 遥测为主。
更低内存上限或清空 Swap 没有消除工作量的证据；先减少不必要的遥测和完整 CLI 周期启动。

初始诊断阶段没有对生产应用变更。NeoStor 要求每次生产部署另获明确批准；候选先完成代码和测试，
再按固定 Git 提交分发，只对批准的服务应用，部署后重新测量。不得借机重启 Vault、封印/解封生产库、
关闭采集功能、删除监控历史或扩大为整套 NeoStor 发版。

watcher 修复已提交并推送 NeoStor：
[`ce2853f`](https://github.com/weironz/neostor/commit/ce2853f0e42252c87842b8f4ac4d639bb3f1c4c8)。
49 项 `ops/releases` 测试通过，Compose 校验及插值一致性通过；实际 BusyBox 1.37.0 合成
HTTP 测试正确解析 200/503/501，无响应连接在 3 秒退出。对生产现存 watcher 仅发一次只读
健康 HEAD，返回 200、wget/exec 均 exit 0，没有执行候选循环、Vault CLI 或真实解封。

对应 [CI](https://github.com/weironz/neostor/actions/runs/38841813143) 的
idc-inventory（包含新回归）、monitoring-config 和 web 已通过；记录时 Rust 总门禁仍在运行。
详细变更、生产应用和回退边界见 NeoStor `docs/49-host-pressure-and-vault-watcher.md`，
已同步到 Mica `greenstor/neostor/49 · 生产主机压力与 Vault 状态轮询修复`。

遥测的最小审查差异为只删除 `scrape_configs` 中 `job_name=ufm-telemetry` 的那个对象，
保留其它所有字段。配置位于 `.34` 专用转发器的 `config/scrape.json`，固定 Compose 来自
`releases/29e3c12de9490d464fd2725ae319da480f5210f8/compose.yaml`，队列卷保持。
其生成模板尚未核实，不能称为已经追溯/修复的 Git 模板；应用前必须保存私有原字节、
owner/mode、摘要和回退配置，并明确记录这个生成来源缺口。随后已获本次生产批准，实际执行见下节。

## 批准后的 Vault watcher 应用与实测

用户明确答复“两项都批准”。原修复 `ce2853f` 的完整 CI 已通过。
预检核对发现运行镜像仍是旧 `1.18`，Compose 已 pin `1.18.5`；本次从固定 Git 下载源码，
只更新 watcher 并保留原 image ID，禁止 pull/build，没有升级 Vault 镜像。
首次重建发现原 256 MiB 运行限制未写入 Compose，命令已回退并立即恢复原限制；
将原资源预算固化到源配置的 `e9940c1` 已推送。后续对等价匿名卷表示的校验误报也已纠正，
最终按实际卷名、路径、目标和读写权限核对，不把 Docker 的表示差异当作换卷。

最终应用时间 **11:33:37**，源码 `e9940c17e1d48249b46e7761d327018e2d721d9a`，
image ID 保留 `sha256:20bce4b1fc21ee5ccc659e3a13fd479894cfffcf650d8ca3c0c4ae7eaecc2c81`。
新 watcher 为 `6aac9c2fd664`，健康 HEAD=200，256 MiB/-1 资源预算、实际挂载和镜像内容均保留，
其余 73 个容器（包含已停止的历史容器）的 ID、启动时间、运行状态未变。
重建继承了 Docker 当前的 3 × 50 MiB json-file 默认轮转，未改 daemon 配置，数据卷未删除。
Vault server 没有重启，也没有主动 seal/unseal 生产凭据库来做测试。

后置静默采样同样丢弃前 20 秒，**11:35:25.592–11:36:37.528，共 71.936 秒**：

| 指标 | 初始静默窗口 | watcher 应用后 |
| --- | ---: | ---: |
| watcher 单核 CPU | 1.642% | 0.0501% |
| watcher 物理读 MiB/s | 0.599 | 0.0053 |
| watcher cgroup 当前内存 MiB | 169.11 | 0.91 |
| watcher 文件页 MiB | 168.01 | 0.48 |
| VictoriaMetrics CPU | 59.789% | 58.0266% |
| VictoriaMetrics 读 / 写 MiB/s | 2.050 / 0.232 | 2.6862 / 1.0025 |
| 主机窗口内 Swap 换入 / 出 MiB | 19.750 / 11.768 | 107.352 / 49.730 |

watcher CPU 下降约 96.9%，物理读下降约 99.1%。当前 cgroup 计费内存下降不等于主机净释放同量，
文件页可能被共享或重新计费。两窗口相隔一小时，外部业务及其它遥测已有变化，
不能把整机或其它服务的变化都归因于 watcher。
后置窗口 VM 仍是主要持续压力源，观察到 max 事件至少增加 891，OOM/OOM-kill 增量为零；
此时 UFM 减量尚未应用。主机可用内存约 1,002.5 MiB，Swap 存量约 2,017.1 MiB。
因此本节只能证明 watcher 本身的工作量减少，不能宣称整机 Swap / I/O 已解决。

私有回退目录为 `/data/neostor/ops/releases/watcher-health-20261016T033334Z`（0700）。
恢复原字节与 owner/mode，继续使用目录中保留原 image ID 及 256 MiB 预算的 override，仅重建 watcher。
原始前后容器配置、操作日志与回执保留主机私有目录；本机脱敏汇总为
`mica-vault-watcher-apply-v2-20261016-receipt.json`、`mica-host-pressure-watcher-after-20261016*.json*`。

UFM 现场已拆成三个任务，后续只删除 `ufm-telemetry`，保留 `ufm-optics` 和 `ufm-rest`；
不能继续沿用旧整包 190,028/180 的减量估计。固定 renderer 来源已进一步核实为
`weironz/ufm-ib-exporter@dec3456b0b13ff8241a7eac5748a7cea75ce129e`。
本机 VPN 断开后，已通过云主机既有 NetBird 网关建立到 `.34` 的通道，并验证原 SSH 主机密钥，
没有切换本机 VPN、修改路由或新建凭据。

用户随后指出“这是 mica 项目”，本轮收窄回 Mica 主机性能，不再执行 Datazone/UFM 变更，
不再要求切换 VPN。中断前该步骤已完成 prepare，只生成私有备份、候选和隔离 dry-run 验证；
没有 apply/reload，没有写 live scrape 或启用消费策略。先前的样本减量只是候选估算，
不能写成生产已减少。NeoStor 与 Mica 同机的资源竞争保留为诊断事实，不将排查自动延伸为
其他项目或机房配置的执行授权。已经应用的同机 watcher 保持现状，未新增回退或其它服务变更。
停止前 prepare 时间为 11:41:01，暂存目录为 `.34` 上的
`ufm-neostor/consumer-policy-backups/20261016T034101546011Z`；目标机还下载了固定工具和
renderer。它们均保留，未安装候选，不把“运行配置未变”表述为“目标机完全没有写入”。

原始脱敏诊断收据保留在本机 Temp：`mica-host-pressure-window-20261016*.json*`、
`mica-host-pressure-quiet-20261016*.json*`、`mica-resident-services-*.json`、
`mica-infisical-swap-limit-*.json`、`mica-rustfs-idle-io-findings-20261016.md`。
原始私有配置、密钥、完整环境变量和生产响应正文不入本报告。
