# 从零拉起一个 Mica 服务端（手动，命令级）

> 2026-07-31 首版。这是**手动**版本，写它是为了给自动化留一份可照抄的底稿 ——
> 一个没人手工走通过的流程，不该先写成脚本。
>
> **这条路径至今没有人端到端走过。** 每一步都标了来源：`[已验证]` 表示该事实在生产上
> 实测过（命令跑过，或配置从运行中的节点抓下来逐条对过）；`[推演]` 表示它由代码与配置
> 推出来、但没在一台新机器上跑过。走完第一遍，请把 `[推演]` 逐条改掉。
>
> 别的文档管别的事：`bootstrap.md` 是**开发机**（Windows/Flutter/MCP），
> `deploy.md` 是把**已有**栈升到某个版本，`backup.md` 是备份与恢复的机制与 runbook，
> `dr-plan.md` 是策略论证（威胁模型、RPO/RTO、为什么不做双活）。本文只管**从无到有**。

## 0. 动手之前要先有的

- 一台干净的 Linux 机器，root 能 SSH 进去
- 三个域名的 DNS 控制权：应用、S3、Traefik dashboard
- **带外密钥**——只有在这台机器要接替旧节点、需要读既有备份时才必须：
  - `RUSTIC_PASSWORD`（备份仓库口令）
  - 阿里云 OSS 的 AccessKey / Secret

`[已验证]` 备份仓库确实由 `RUSTIC_PASSWORD` 加密，而节点上那份 `.env` 是它唯一的副本 ——
所以它不能"在新机器上重新生成"，必须带过来。全新实例可以先不接备份，之后再补。

## 1. DNS 先行 `[推演]`

在碰机器之前把 A 记录指过去，并等它生效：

```bash
dig +short mica.example.com
dig +short s3.mica.example.com
dig +short traefik.example.com
```

三条都要回新机的公网 IP。**为什么必须最先做**：Traefik 用 ACME 的 **TLS-ALPN-01** 挑战签
证书（`--certificatesresolvers.letsencrypt.acme.tlschallenge=true`，`[已验证]`，从运行中的
节点抓下来），域名解析不到这台机器就签不出证书 —— 而失败的样子是安静地烧重试次数，不是
报错。

## 2. Docker `[推演]`

```bash
curl -fsSL https://get.docker.com | sh
docker compose version    # 必须有 v2:整套栈用 `docker compose`,不是 `docker-compose`
```

## 3. 目录与网络 `[已验证目录名 / 推演命令]`

```bash
mkdir -p /data/mica /data/traefik
docker network create traefik-network
```

`[已验证]` 生产上两套栈的工作目录就是 `/data/mica` 与 `/data/traefik`，网络名
`traefik-network`。

**这个网络必须带外建**：两份 compose 都把它声明成 `external: true`，谁都不拥有它。先起
Mica 会直接报网络不存在；而如果让某一方拥有它，那一方 `down` 时会带走另一方的网络。

## 4. Traefik `[已验证配置 / 推演流程]`

```bash
scp deploy/traefik/docker-compose.yml root@<node>:/data/traefik/
scp deploy/traefik/.env.example        root@<node>:/data/traefik/.env
```

填 `/data/traefik/.env`（每个键的含义写在文件注释里）。dashboard 的 basic auth：

```bash
# 注意最后那段 sed:compose 读 .env 时 `$` 必须写成 `$$`,否则插值会吃掉
# bcrypt 哈希的一部分,登录失败的样子和「密码错了」一模一样
htpasswd -nbB traefikadmin '<password>' | sed 's/\$/\$\$/g'
```

起它：

```bash
cd /data/traefik && docker compose up -d
docker compose ps        # traefik-traefik-1 应当 healthy(它自带 /ping healthcheck)
```

`[已验证]` 这份 compose 是 2026-07-31 从运行中的节点抓下来的，22 个 command 参数与
`docker inspect` 逐条一致。

## 5. Mica 栈 `[推演]`

```bash
scp deploy/docker-compose.yml root@<node>:/data/mica/
scp deploy/.env.prod.example  root@<node>:/data/mica/.env
```

填 `/data/mica/.env`。**密钥分两类，这是整个流程最容易犯、后果最重的错：**

| 类别 | 变量 | 怎么来 |
| --- | --- | --- |
| 新生成 | `JWT_SECRET`、`POSTGRES_PASSWORD`、`S3_ACCESS_KEY`、`S3_SECRET_KEY` | `openssl rand -hex 32` |
| **必须带过来** | `MICA_BACKUP_PASSWORD`（即 `RUSTIC_PASSWORD`）、`OSS_ACCESS_KEY_ID`、`OSS_SECRET_ACCESS_KEY` | 带外（第 0 节） |

把 `MICA_BACKUP_PASSWORD` 当成"随机生成"的一员，机器照样起得来 —— 但**你再也打不开自己的
备份**，恢复流程第一步就走不下去。

其余必填：

```
MICA_REGISTRY=registry.cn-shenzhen.aliyuncs.com/willspace
MICA_VERSION=v0.13.6                       # 钉一个已发布版本,不要用 latest
MICA_APP_BASE_URL=https://mica.example.com
MICA_REGISTRATION_ENABLED=                 # 留空 = 关闭(首账号例外仍放行)
```

起栈：

```bash
cd /data/mica && docker compose up -d
docker compose logs -f api | head -50      # 迁移在 api 启动时跑(sqlx::migrate!)
curl -s https://mica.example.com/api/ready
# 期望 {"status":"ready",...,"version":"0.13.6","registration_open":true}
```

**验 `/api/ready` 而不是 `/api/health`** `[已验证]`：只有 ready 摸数据库，health 在库挂掉
时照样报 ok。此刻 `registration_open` 应为 `true` —— 空实例的首账号例外（下一步）。

> ⚠️ **一处我没有确认过**：RustFS 的 bucket（`S3_BUCKET=mica`）是启动时自动创建，还是要
> 先手工建。第一次走这条路时请留意；如果上传图片报 bucket 不存在，就是它，把建桶命令补
> 进这一节。

## 6. 第一个账号 `[已验证机制 / 推演流程]`

注册默认关闭，但**空实例的第一个账号永远放行**、且直接标记为已验证 —— 否则全新自托管装
不起来。所以直接在登录页注册即可（客户端读 `/api/ready` 的 `registration_open`，此时会显
示注册入口）。

建完之后再查一次，确认门关上了：

```bash
curl -s https://mica.example.com/api/ready | grep -o '"registration_open":[a-z]*'
# 期望 "registration_open":false
```

## 7. 接上日常 CD `[已验证脚本 / 推演用户部分]`

**这一步是 provisioning 与日常部署的接缝。不做，这台机器只能永远手工部署。**

```bash
useradd -m -s /bin/bash mica-deploy
usermod -aG docker mica-deploy
mkdir -p ~mica-deploy/.ssh && chmod 700 ~mica-deploy/.ssh
```

把 CI 的公钥写进去，**用 `command=` 钉死** —— 这是整套部署安全模型的核心：CI 只能执行
`deploy <version> <sha>`，别的做不了。

```bash
cat >> ~mica-deploy/.ssh/authorized_keys <<'EOF'
restrict,command="/usr/local/sbin/mica-deploy" ssh-ed25519 AAAA... ci-deploy-key
EOF
chown -R mica-deploy:mica-deploy ~mica-deploy/.ssh
chmod 600 ~mica-deploy/.ssh/authorized_keys
```

装策略脚本本体（在**你自己的机器**上跑，它会 ssh 进节点）：

```bash
just sync-deploy-script origin/main
```

`[已验证]` 这个 recipe 存在：它把 `deploy/mica-deploy.sh` 装到 `/usr/local/sbin/mica-deploy`，
先 `bash -n` 过一遍再原子替换，并打印两端 sha256 供比对。

之后日常升级走 `deploy.md`（推 tag → CI → 触发 Deploy workflow）。**注意**：只要
`deploy/docker-compose.yml` 变过，节点的指纹校验就会拒绝 Deploy workflow，必须走
`just deploy-prod <version>` —— 这不是故障，是设计：一个版本不能配着不属于它的 compose
部署。

## 8. 接上备份 `[已验证]`

三条腿，都配在同一份 `/data/mica/.env`：

```
MICA_BACKUP_TOKEN=<一个只读的 Mica PAT>
MICA_BACKUP_PGURL=postgres://mica:<POSTGRES_PASSWORD>@postgres:5432/mica
OSS_BUCKET= OSS_ENDPOINT= OSS_REGION= OSS_ACCESS_KEY_ID= OSS_SECRET_ACCESS_KEY=
RUSTFS_S3_ACCESS_KEY_ID=<同 S3_ACCESS_KEY>
RUSTFS_S3_SECRET_ACCESS_KEY=<同 S3_SECRET_KEY>
OSS_BLOB_BUCKET=
HEALTHCHECK_URL=<healthchecks.io 之类的死人开关>
```

backup 容器在 `backup` profile 里，默认不起。起它并**一次性**初始化 rustic 仓库：

```bash
cd /data/mica && docker compose --profile backup up -d backup
docker exec mica-backup-1 rustic init                      # 只做一次;已有仓库则跳过
docker exec mica-backup-1 /usr/local/bin/mica-backup.sh    # 手动跑一次,别等凌晨
```

验证三条腿真的都落了：

```bash
docker exec mica-backup-1 rustic snapshots --filter-label _pgdump | tail -3
docker logs mica-backup-1 2>&1 | grep -E 'pg_dump|rclone|snapshotted|WARN'
```

**不要只看它退出码为 0。** `MICA_BACKUP_PGURL` 没设时脚本会按设计降级、打一条 WARN、把内容
备完、正常退出 —— 死人开关照样是绿的。这个洞在生产上活了几个月，直到 2026-07-30 才被发现
（`dr-plan.md` §1.1）。**必须亲眼看到 `label=_pgdump` 有快照。**

## 9. 验收清单

全部通过才算这台机器起来了：

```bash
curl -s https://mica.example.com/api/ready                          # ready + 版本正确
curl -so /dev/null -w '%{http_code}\n' https://mica.example.com/    # 200
```

- [ ] 用第 6 步的账号能登录
- [ ] 新建一页、写几个字、刷新后内容还在
- [ ] 传一张图，刷新后图还在（证明 RustFS 与 `files` 表接通了）
- [ ] `rustic snapshots --filter-label _pgdump` 有今天的快照
- [ ] 证书是 Let's Encrypt 签的、不是自签（浏览器不报警）
- [ ] `docker compose ps`：5 个 mica 服务 + traefik 全部 healthy / running

## 10. 已知的空白

- **整条路径没人走过**，所以耗时未知 —— 这正是 `dr-plan.md` 里 RTO 那格空着的原因。走完
  第一遍请把耗时记回这里，那个数字就是 RTO 的下限。
- **RustFS bucket 是否需要预先创建**：未确认（见第 5 节的 ⚠️）。
- **恢复既有数据**不在本文范围：那是 `backup.md` 的"从 pg_dump 恢复"加对象字节还原。动手前
  先读 `dr-plan.md` 里那条 —— **PG 恢复与 markdown 重导是互斥的两条路，不是互补的两半**
  （markdown 重导会把图片 re-host 成新 key，与恢复回来的 `files` 表对不上）。
- 第 2、7 步的用户与目录部分是**推演**，第一次走最可能卡在这里。
