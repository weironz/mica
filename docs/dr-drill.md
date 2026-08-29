# 容灾恢复演练 —— 手工走一遍(工作表)

> **✅ 2026-08-29 全程走通,每一步都是 `[已验证]`。** RTO 第一个真数字:**≈ 10 分钟**。
>
> 原来这里写着「每一步都标着 `[推演]`,意思是写下来的时候没人走过」。走完之后那句话
> 可以删了 —— 但**走的过程中撞上四个会让流程停死的问题**(见文末),而它们在文档里
> 全都是"假设成立"的样子。这份文档现在才第一次配得上叫 runbook。
>
> **下次再走,仍然按这份走,并且仍然预期会撞上新的东西。** 演练的价值不是确认它能用,
> 是找出它哪里不能用。
>
> **为什么必须先手工走**(`cd-plan.md` §6 早就写死了):*一个没人手工走通过的流程,不该
> 先写成脚本 —— 没走过就写等于把推演固化成代码,而且脚本一旦存在,人就不再读文档了,
> 错误会被放大。*
>
> **产出物是一个数字**:全程累计耗时 = RTO 的第一个真实值。`dr-plan.md` §3 那格空到今天。

## 演练前:三条纪律

1. **用测试子域,不要用 `mica.cloudcele.com`。** Let's Encrypt 对同一组域名每周只发
   5 张重复证书,反复演练会撞上限 —— 届时机器起来了却**没有 TLS**,而那是真出事时最
   不需要遇到的问题。本次用 `mica-dr.cloudcele.com` + `mica-s3.dr.cloudcele.com`,
   **由 tofu 建**(`dr/aliyun/dns.tf`),`destroy` 时一起收走。
2. **不碰生产。** 全程只读生产的**备份仓库**(OSS),不 ssh 生产、不改生产 DNS。
3. **每步记时间。** 手机秒表或 `date` 都行,填进下面的表。

## 演练前:把这些准备好(缺一样就会卡在半路)

> **完整的前置条件清单在 [`dr-plan.md` §9](dr-plan.md)** —— 那一节收的是「平时为真、
> 用时才验」那一类:余额、GitHub Secret、私钥、备份新鲜度、域名控制权。每条配一条
> 可执行的检查命令。下表是这次演练直接要用到的那几样。

| 需要 | 从哪来 | 缺了会怎样 |
| --- | --- | --- |
| 测试域名两条 A 记录 | 你的 DNS | ACME 签不出证书 |
| `RUSTIC_PASSWORD` | 密码管理器(节点 `.env.secrets` 里叫 **`MICA_BACKUP_PASSWORD`**) | **拿到一个完好但永远打不开的仓库** |
| OSS 读凭据 | 密码管理器 | 读不到备份 |
| **三个**:`MICA_BACKUP_PASSWORD` + `OSS_ACCESS_KEY_ID` + `OSS_SECRET_ACCESS_KEY`(后两个在 GitHub Secrets 里叫 `ALICLOUD_*`,同一把 key) | 密码管理器 **+ GitHub Secrets** | 读不到备份仓库(口令负责解密,OSS 那对负责连得上)。**其余 10 个键 v0.13.40 起在备份里**(`_config` lineage);这三个不可能在里面 —— 它们就是开锁的那套。进 Secrets 是为了恢复链路**不依赖人在场** |
| 阿里云可用额度 **≥ 100 元** | 充值 | **开不出按量付费实例**——实测拦在这里(见步骤 0) |

`.env.secrets` 的键名(值不在这里)。**演练不需要生产的原值** —— 除了
`MICA_BACKUP_PASSWORD` 和 OSS 读凭据,其余都可以当场 `openssl rand` 生成:它们只决定
这台机器**怎么初始化自己**,不影响能不能把生产数据读回来:

```
JWT_SECRET  POSTGRES_PASSWORD  S3_ACCESS_KEY_ID  S3_SECRET_ACCESS_KEY
MICA_BACKUP_TOKEN  MICA_BACKUP_PASSWORD  OSS_ACCESS_KEY_ID  OSS_SECRET_ACCESS_KEY
MICA_MAIL_ACCESS_KEY_ID  MICA_MAIL_SECRET_ACCESS_KEY  MICA_BACKUP_PGURL
RUSTFS_S3_ACCESS_KEY_ID  RUSTFS_S3_SECRET_ACCESS_KEY
```

> ⚠️ `RUSTFS_S3_*` 必须与 `S3_*` **相同** —— 只有一个 RustFS,它只认一对 key。
> 两套名字曾经漂移,rclone 因此 403 了很久(`dr-plan` §8 #4)。v0.13.39 起 compose
> 默认让前者取后者,所以**新装机可以整个不填 `RUSTFS_S3_*`**。

非机密配置(可直接抄):

```
MICA_REGISTRY=registry.cn-shenzhen.aliyuncs.com/willspace
OSS_BUCKET=mica-backup-cloudcele        OSS_BLOB_BUCKET=mica-backup-cloudcele
OSS_ENDPOINT=https://oss-cn-shenzhen.aliyuncs.com
OSS_REGION=oss-cn-shenzhen              OSS_ROOT=mica
BACKUP_HOUR=3                           TZ=Asia/Shanghai
TRAEFIK_IMAGE_TAG=traefik:v3.6.10
DOMAIN=<测试域名>                        S3_DOMAIN=s3.<测试域名>
MICA_VERSION=v0.13.39
```

---

## 步骤

### 0. `[已验证 2026-08-29]` 开一台机器

**已经写成 OpenTofu 了**:[`dr/aliyun/`](../dr/aliyun/README.md)。

```powershell
setx ALICLOUD_ACCESS_KEY "..." ; setx ALICLOUD_SECRET_KEY "..."   # 设一次
```

⚠️ **`setx` 只对新开的终端生效** —— 关掉当前窗口重开一个,否则 `tofu plan` 会说没凭据。

```bash
cd dr/aliyun && tofu init && tofu plan     # 先看一眼再花钱
tofu apply && tofu output
```

⚠️ **先查余额**。按量付费要求可用额度 **≥ 100 元**,不够就在 apply 的最后一步报
`InvalidAccountStatus.NotEnoughBalance`(2026-08-29 首次 apply 实测:余额 81.93,
网络层全建好、只有实例失败)。生产机是包年包月,余额低**不影响它跑** —— 所以这条
在真出事之前完全不可见。

```bash
aliyun bssopenapi QueryAccountBalance --region cn-shenzhen
```

失败后**不用清理**:VPC / 交换机 / 安全组 / 密钥对都不计费,充值后再 `tofu apply`
会认出它们已存在,只补那台机器。

> 本机先 `pwsh` 进 PowerShell 7 —— Windows PowerShell 5.1 与 cmd 不认 `&&`。

完整版(含装工具、公钥、destroy)见 [`dr/aliyun/README.md`](../dr/aliyun/README.md)
的「快速开始」。

`tofu output chosen` 会给出云**实际**选中的可用区/规格/镜像 —— 抄进下表的「实际」栏。

> 只有这一步被脚本化了,第 1 步之后仍然手工走。`cd-plan` §6 那条「没走过别写脚本」
> 针对的是**恢复逻辑**;建机器这件事恰恰适合声明式 —— `plan` 能在花钱前看一眼,
> `destroy` 能保证不留计费尾巴。
>
> 想用控制台手开也完全可以,下表就是要点。**每一个选择都对应 `.tf` 里的一行**,
> 所以两条路填的是同一张表。

| 项 | 选什么 | 为什么 | 实际 |
| --- | --- | --- | --- |
| **地域** | **深圳(cn-shenzhen)** | 必须和备份桶同地域(`OSS_ENDPOINT` 是 `oss-cn-shenzhen`)。跨地域拉 1.5 GB 又慢又要流量费 | cn-shenzhen-b |
| 付费方式 | **按量付费** | 演练完就删;包年包月删不掉只能退 | 按量付费 |
| 规格 | **2 vCPU / 4 GiB**(如 `ecs.e-c1m2.large`) | 生产是 2 核 3.5G,演练要能代表生产 | `ecs.c5.large`(2C4G) |
| 镜像 | **Ubuntu 26.04 LTS x64** | tofu 里是变量 `image_name_regex`,按名字现查而非写死 id | ubuntu_26_04_x64_20G_alibase_20260810 |
| 系统盘 | **40 GiB** ESSD | 要装下:docker 镜像 ~4G + 恢复的数据 ~1.5G + 中途那份 634 MB 明文 dump。生产 `/data` 是独立 100G 盘,但那 69G 大半不是 mica 的 —— **mica 的卷加起来只有 1.5 GB**(postgres 439 MB + rustfs 1.01 GB) | 40G ESSD,装完系统占 2.6G |
| 数据盘 | **不要** | 演练不需要;真恢复时按生产形态挂一块再 `mkdir /data` 即可 | 无 |
| 公网 IP | 勾选**分配公网 IPv4**,按流量计费 | ACME 与你的浏览器都要够得着 | 47.106.118.86 |
| 带宽 | 峰值 5 Mbps 以上 | 要从 OSS 拉 1.5 GB;太小就是干等 | 5 Mbps 按流量 |
| 安全组 | 放行 **22 / 80 / 443** 入方向 | 80 与 443 是 ACME 与访问;少 443 证书签不出来 | 22/80/443 已放行 |
| **密钥对** | **在创建页就选中** | ⚠️ **先有鸡还是先有蛋**:不在创建时注入公钥,机器起来你就进不去,也没法用 ansible。这是整个流程里最容易漏、且**事后补不了**的一步 | tofu 创建时注入,免密登录成功 |

开完记下公网 IP:`47.106.118.86`

```bash
ssh root@<公网IP> 'cat /etc/os-release | head -2; nproc; free -h | head -2; df -h /'
```

**判据**:能免密登录(用你选的密钥对),规格与上表一致。
**实际**:`Ubuntu 26.04 LTS` / 2 核 / 3.4Gi / `/dev/vda3 40G`,免密登录成功。
**耗时**:**约 1 分钟**(`tofu apply` 16s,其中实例本身 12s;之后 sshd 还要 ~20s 才接受连接
—— 前两次 `ssh` 都是 `Connection closed`,**这不是失败,是没起完**,别据此去查安全组)。

> 💰 **演练完当天就删** —— 实例和**弹性公网 IP 要分别删**,只删实例会留下一个还在计费
> 的 EIP。失败的演练最容易留这种尾巴。

### 1. `[已验证 2026-08-29]` DNS 先行 —— 顺序错了会静默烧掉签发次数

把测试域名的两条 A 记录指向新机器 IP,**在起 Traefik 之前**。ACME 用 TLS-ALPN-01
挑战打 :443,域名没解析过来就签不出证书,而它**不会响亮地失败**,只会安静地重试。

**这一步也归 tofu 了**(`dns.tf`):记录跟机器同生共死,`destroy` 不会留下一条指向
已删实例的野记录 —— 那种记录不报错,只是安静地指向后来拿到这个 IP 的别人。

```bash
dig +short mica-dr.cloudcele.com          # 应返回新机 IP
dig +short mica-s3.dr.cloudcele.com
```

Windows 上没有 `dig`,用 `Resolve-DnsName <名字> -Server 223.5.5.5`(指定权威侧的
公共解析器,绕开本机缓存)。

**判据**:两条都返回新机 IP 才继续。
**实际**:两条都 → `47.106.118.86`。
**耗时**:**秒级**(`tofu apply` 建记录 1s,阿里云 DNS 立即可查)。

> ⏱️ **但 TTL 是 600 秒**,而阿里云**免费版解析的下限就是 600** —— 填更小会被 API 拒。
> 这里不影响(新名字没有旧缓存),但真恢复时改的是**已经在用的** `mica` 记录,
> 那 10 分钟传播尾巴要算进 RTO。要更快只能上付费版解析,或者别靠 DNS 切换。
>
> ⚠️ 真恢复时改 `mica` 那条记录**不要交给这份 tofu** —— 归 tofu 管就意味着
> `destroy` 会删掉生产解析。`dns.tf` 里写了这条警告。

### 2-3. `[已验证 2026-08-29]` 底座 + Traefik —— **一条 ansible 命令**

原本这两步是手打的,理由是「`cd-plan §6`:没人手工走过的流程不该写成脚本」。走完
之后那条理由就用完了 —— 它针对的是**恢复逻辑**(第 6、7 步),而装 Docker、起
Traefik 是「配」那一层,本来就该是 ansible 的活(`dr-plan §7.2.2` 的分层)。

```bash
DR_HOST=$(cd dr/aliyun && tofu output -raw public_ip) ansible-playbook -i ansible/inventory.yml ansible/provision.yml -e target=mica-dr
```

⚠️ **控制端不能是 Windows**(ansible 装得上、一跑就 `WinError 87`)。本机没有通用
WSL 发行版,所以借容器当控制端 —— 已经封成脚本,**不用每次重造**:

```bash
DR_HOST=$(cd dr/aliyun && tofu output -raw public_ip) sh scripts/ansible-in-docker.sh ansible/provision.yml -e target=mica-dr
```

它做三件事:把公钥复制出来 `chmod 600`(Windows 挂进来的权限是 0777,ssh 会拒绝用)、
装 `ansible/requirements.yml` 里的 collection、跑 playbook。

> `scripts/deploy-prod.sh` 在 Windows 上是**拒绝**并让你走 CI —— 对于发版那是对的。
> 但**恢复时不是**:你可能正站在一台笔记本前、面对一个坏掉的节点,没心情绕一圈 GitHub。
> 这个脚本用已经装好的 Docker,十秒钟给你一个控制端。

**判据**:`PLAY RECAP` 里 `failed=0`;traefik 容器 healthy;`curl https://traefik.<域名>/`
返回 **401**(而不是连接错误)—— 401 一次证明两件事:证书签出来了、basic auth 在生效。

**实际**:`ok=17 changed=9 failed=0`;traefik `Up (healthy)`;dashboard 401、TLS 校验通过、
日志 0 条 error。
**耗时**:**约 2 分钟**(含经代理拉三个第三方镜像)。

> **这一步找到了三个只有真跑才会知道的东西**,全部已固化进 `provision.yml`:
>
> 1. **`curl get.docker.com | sh` 在国内 ECS 上直接失败** —— `download.docker.com`
>    连接重置。改走阿里云 docker-ce 镜像。codename 用 `ansible_distribution_release`
>    现取而不写死:26.04 是 `resolute`,而容灾当天没人记得这个。
> 2. **🔴 这台机器根本连不上 `registry-1.docker.io`** —— `postgres` / `rustfs` /
>    `traefik` 三个第三方镜像一个都拉不到。**也就是说在今天之前,「给你一台新机器就能
>    重建」这个前提是假的:栈起不来。** 生产之所以没事,只因为那些镜像早就在它上面了。
>    → `provision.yml` 现在配 pull-through 代理,并**显式** pull 这三个镜像:失败硬停,
>    而不是让 compose 隐式拉、再报一堆看不懂的服务错误。用代理而非自建私有副本是
>    用户拍的板 —— 私有副本每次上游升版都要人工同步,而「记得同步」正是会被跳过的
>    那类步骤;代价是供应链信任,所以才要求它停得响。
> 3. **traefik `.env` 里 htpasswd 哈希的 `$` 会被 compose 当变量吃掉** ——
>    `The "apr1" variable is not set`。凭据不是坏得响亮,是被**悄悄改错**。模板里 `$` → `$$`。

### 4. `[已验证 2026-08-29]` 还原凭据 —— **必须在起栈之前**

> **这一步是 8-29 当天补出来的,而且是补在错的位置之后。** 那天的实际顺序是先起栈、
> 再手工填凭据,走得通只是因为演练机的库是用临时口令新建的。**真恢复时这个顺序是坏的**:
> `POSTGRES_PASSWORD` 在 `.env.secrets` 里,postgres 用它 initdb —— 先起栈就是
> 「库用旧口令建的、配置写着新口令」,而它不会当场报错,只会在某个连接时刻才显形。

新机器上只有镜像,所以还原不能依赖一个已经跑起来的栈(否则又是先有鸡还是先有蛋)。
`mica-cli backup restore-config` 就是为这一刻加的(v0.13.41):

```bash
docker run --rm -v /data/mica:/restore --entrypoint /usr/local/bin/mica-cli \
  -e RUSTIC_PASSWORD -e OSS_BUCKET -e OSS_ENDPOINT -e OSS_REGION -e OSS_ROOT \
  -e OSS_ACCESS_KEY_ID -e OSS_SECRET_ACCESS_KEY \
  registry.cn-shenzhen.aliyuncs.com/willspace/mica-cli:v0.13.41 backup restore-config
install -m 600 /data/mica/etc/mica/env.secrets /data/mica/.env.secrets
```

⚠️ `--entrypoint` 不能省:镜像 ENTRYPOINT 是整条 `mica-cli backup daemon`。

**这三个环境变量从哪来**:GitHub Secrets(`dr-plan §2.2`)。它们是恢复链上仅剩要被
「携带」的秘密 —— 其余 10 个键从这一步还原回来。

> 🔴 **这一步之后,演练机持有生产凭据** —— 真的 JWT secret、S3 keys、邮件 AK,躺在
> 一台公网机器上。这不是设计缺陷,这就是真恢复要做的事(不这么做就没验证任何东西),
> 但**演练的性质因此改变了**:在这一步之前它是一台空机器,之后它是一份生产凭据的副本。
>
> 所以:**把 `ssh_cidr` 收窄到你的出口 IP**(`dr/aliyun/variables.tf` 里就有这个变量,
> 默认全开只是为了演练顺手),而且**当天必须 destroy**。如果演练机在网上过了夜,
> 正确处置不是"应该没事",是**轮换那批凭据**。

**判据**:`.env.secrets` 里的键数与生产一致,且 `POSTGRES_PASSWORD` 是生产的原值。
**实际**:**15 个键全部还原**(比预期多两个 —— 生产的 `.env.secrets` 里还有
`CORTEX_MAIL_*`,是同机另一个应用的)。**输入只有三个秘密**。
**耗时**:**2 秒**(1.0 KiB)。

> **这条闭环成立了**:生产把凭据备到异地 → 新机器只凭三个携带的秘密就把整套拿回来。
>
> 走这一步之前先做了一次半程验证,值得抄:用一把能读桶但**故意给错口令**的凭据跑一次,
> 报的是 `The password that has been entered, seems to be incorrect` 而**不是"连不上"**
> —— 于是"OSS 那一半通了"和"解密这一环没验"被分开了,剩下的未知只有一个。

### 5. `[已验证 2026-08-29]` 起栈 —— **复用 `deploy.yml`,不另写一套**

> 🔴 **顺序陷阱,实测撞上过**:`deploy.yml` 会把 **api 一起起来**,而 api 启动就跑
> `sqlx::migrate!` —— 于是**库不再是空的,schema 已经建好了**,下一步的 `pg_dump`
> 灌不进去:
>
> ```
> ERROR:  type "object_type" already exists
> ```
>
> 这不是某条命令写错了,是**这份文档的步骤顺序本身是错的**。真恢复要么
> **只起 postgres → 灌库 → 再起 api**,要么像本次这样在灌之前把库重建成空的:
>
> ```bash
> ./dc stop api web
> docker exec mica-postgres-1 psql -U mica -d postgres >   -c 'DROP DATABASE IF EXISTS mica WITH (FORCE);' -c 'CREATE DATABASE mica OWNER mica;'
> ```
>
> 灌完再 `./dc start api web`:那时 `_sqlx_migrations` 里已经有记录,迁移不会重跑。
>
> 〔为什么推演时看不出来:写文档的人知道"迁移随 api 启动跑",也知道"pg_dump 含
> schema",但没有把这两件事放在一起。**只有真的按顺序执行一遍,它们才会相撞。**〕

原计划是手工写 `.env` / 取 compose / `docker compose up`。不该这么做:**一条不是
日常流程的恢复流程,是一条没人测过的流程**。日常发版跑的就是 `ansible/deploy.yml`,
它已经会做全部三件事(从 tag 取 compose、按 host_vars 渲染 `.env`、起栈)。

```bash
git show v0.13.41:deploy/docker-compose.yml > /tmp/compose.yml
DR_HOST=<IP> ansible-playbook -i ansible/inventory.yml ansible/deploy.yml -e target=mica-dr -e version=0.13.41 -e compose_src=/tmp/compose.yml
```

**为此改了 `deploy.yml` 两处**(都不削弱它对生产的保护):

1. `hosts` 从写死 `mica-prod` 改成 `{{ target | default('mica-prod') }}` ——
   `scripts/deploy-prod.sh` 什么都不传,行为一字不变。
2. **它以前拒绝全新节点**,而且拒得很难看:`slurp` 读不到 `.env` 就直接炸,断言根本
   没机会说话。现在先 `stat` 分辨两种情况 —— **「压根没有 `.env`」(全新机器)** 与
   **「`.env` 在但丢了 `MICA_VERSION`」(被手改坏的老节点)**,只放行前者。
   后者仍然拒绝,因为那条规则本来就是为它写的。同时全新节点**不取还原点**:没有库
   可 dump,取了只会在 `docker exec mica-postgres-1` 上失败。

> 这是这次演练**改动生产路径**的唯一一处,而它恰恰是最该改的:容灾要走的那条路,
> 在今天之前从来没被走过,于是"能重建"只是推断。

**判据**:`PLAY RECAP` `failed=0`;`./dc ps` 全 running;`/api/health` 报对版本。
**实际**(第二次,凭据还原之后、且**删掉 `.env` 走真正的全新节点路径**):
`ok=31 changed=5 failed=0`,**43 秒**。
第一次(第 4 步之前跑的)也是 `failed=0`,但那个顺序是坏的 —— 见第 4 步开头的警告。

> `localhost:8080` 空响应是对的 —— api 不发布到宿主端口,只经 traefik 进出。
> 别把它当成故障。

### 6. `[已验证 2026-08-29]` 灌数据库

> 🔴 **绝不要 `./dc --profile backup up -d backup`。** 那会启动 backup **daemon**,
> 它 run-on-start 立刻**往生产的备份仓库写**,并且跑 `forget --prune` ——
> **在演练机上 prune 掉生产的快照**。演练纪律「只读生产的备份仓库」在这里是硬约束,
> 不是客气话。用一次性容器取:

```bash
IMG=registry.cn-shenzhen.aliyuncs.com/willspace/mica-cli:v0.13.41
docker run --rm -v /data/mica/.env.secrets:/s:ro -v /data/mica/.env:/e:ro -v /data/restore:/restore \
  --entrypoint sh "$IMG" -c '
    export $(grep -E "^OSS_" /e /s -h | xargs)
    RUSTIC_PASSWORD=$(grep "^MICA_BACKUP_PASSWORD=" /s | cut -d= -f2-); export RUSTIC_PASSWORD
    # 借 restore-config 渲染 /etc/rustic/rustic.toml(那份配置唯一的权威来源),
    # 再在同一个容器里复用它 —— 不另抄一份 toml 进 runbook。
    mica-cli backup restore-config --out /tmp/t >/dev/null 2>&1 || true
    rustic restore latest /restore --filter-label _pgdump'
```

⚠️ **`OSS_BUCKET` 等非机密项在 `.env`,不在 `.env.secrets`** —— 只挂后者会得到
`error: OSS_BUCKET is required for the backup`,而那句话听起来像凭据缺失。两个都挂。

**先验证 dump 完整再灌**(判据来自 `deploy.yml` 的还原点门禁,这里同一套):

```bash
D=/data/restore/var/lib/mica/export/_pgdump/mica.sql
grep -c '^COPY public\.' "$D"                        # >= 21
grep -c 'PostgreSQL database dump complete' "$D"     # 1,证明没被截断
```

**库必须是空的**(见第 5 步开头的顺序陷阱):

```bash
./dc stop api web
docker exec mica-postgres-1 psql -U mica -d postgres \
  -c 'DROP DATABASE IF EXISTS mica WITH (FORCE);' -c 'CREATE DATABASE mica OWNER mica;'
docker exec -i mica-postgres-1 psql -U mica -d mica -v ON_ERROR_STOP=1 -q < "$D"
./dc start api web
```

**判据**:`COPY public.` 段数 ≥ 21;导入 ERROR 数 = 0;行数与生产一个量级。
**实际**:21 段;**取快照 9.2s(634 MB)、导入 16s、0 个 ERROR**;
`users=2 workspaces=32 views=17799 docs=14881 migrations=26`;
`./dc start api web` 之后 api healthy、`/api/ready` 200,**迁移没有重跑**。
**耗时**:**约 1 分钟**(含验证)。

> 顺带验掉一条以前没人验过的:**pg18 的 dump 里仍然有 `deploy.yml` 还原点门禁 grep 的
> 那两个标记**(`PostgreSQL database dump complete` / `COPY public.document_yrs_base`)。
> pg16→18 升级之后那道门禁一直没被真正触发过(最近几版都没有新迁移),所以它成不成立
> 此前只是假设。

### 7. `[已验证 2026-08-29]` 灌对象字节(图片)

**这里有个缺口**:`mica-cli` 只有**写**镜像的路径,没有把对象**取回来**的命令 ——
`render_rclone_conf` 只在 backup run 里被调用,而那条路在演练机上是禁区(见第 6 步的
红字)。所以这次用 rclone 的环境变量式 remote 绕过去,**它是第二处表示**,该补一个
`mica-cli backup restore-objects`(记在 `roadmap.md`)。

```bash
IMG=registry.cn-shenzhen.aliyuncs.com/willspace/mica-cli:v0.13.41
g() { grep -m1 "^$1=" "$2" | cut -d= -f2-; }
S=/data/mica/.env.secrets; E=/data/mica/.env
docker run --rm --network mica_default --entrypoint rclone \
  -e RCLONE_CONFIG_OSSBLOB_TYPE=s3 -e RCLONE_CONFIG_OSSBLOB_PROVIDER=Alibaba \
  -e RCLONE_CONFIG_OSSBLOB_ENDPOINT="$(g OSS_ENDPOINT $E)" \
  -e RCLONE_CONFIG_OSSBLOB_ACCESS_KEY_ID="$(g OSS_ACCESS_KEY_ID $S)" \
  -e RCLONE_CONFIG_OSSBLOB_SECRET_ACCESS_KEY="$(g OSS_SECRET_ACCESS_KEY $S)" \
  -e RCLONE_CONFIG_RUSTFS_TYPE=s3 -e RCLONE_CONFIG_RUSTFS_PROVIDER=Other \
  -e RCLONE_CONFIG_RUSTFS_ENDPOINT=http://rustfs:9000 \
  -e RCLONE_CONFIG_RUSTFS_FORCE_PATH_STYLE=true \
  -e RCLONE_CONFIG_RUSTFS_ACCESS_KEY_ID="$(g RUSTFS_S3_ACCESS_KEY_ID $S)" \
  -e RCLONE_CONFIG_RUSTFS_SECRET_ACCESS_KEY="$(g RUSTFS_S3_SECRET_ACCESS_KEY $S)" \
  "$IMG" copy ossblob:mica-backup-cloudcele/mica-blobs rustfs:mica --transfers 8 --stats 30s
```

⚠️ 容器必须 `--network mica_default`,否则解析不到 `rustfs`。

**判据**:`rclone size` 两端的**对象数与字节数都相等**。只看"传完了没报错"不够 ——
`copy` 成功但传了 0 个对象也是"没报错"。

```bash
rclone size ossblob:mica-backup-cloudcele/mica-blobs
rclone size rustfs:mica
```

**实际**:两端都是 **6467 个对象 / 994.201 MiB(1042495633 字节)**,逐字节相等。
**耗时**:**不到 2 分钟**(同地域 OSS → ECS)。

> 403 打在**源**还是**目的**,含义完全不同:目的(`rustfs:`)= 本机那对凭据不对;
> 源(`ossblob:`)= OSS 凭据不对。看清报错里的 bucket 名再动手。

### 8. `[已验证 2026-08-29]` 核验(版本号证明不了数据回来了)

**先程序化对一遍**,再用浏览器看 —— 前者能给出"哪里不一致",后者只能给出"看起来不对"。

```bash
Q="select 'users='||(select count(*) from users)||' workspaces='||(select count(*) from workspaces)||' members='||(select count(*) from workspace_members)||' views='||(select count(*) from views)||' docs='||(select count(*) from document_yrs_base)||' nonempty='||(select count(*) from document_yrs_base where length(state)>0 and content_text<>'')||' comments='||(select count(*) from comments);"
# 在生产和演练机上各跑一次,逐字对比
docker exec mica-postgres-1 psql -U mica -d mica -tAc "$Q"
```

**实际**(两端完全相同):

```
users=2  workspaces=32  members=32  views=17799  docs=14881  nonempty=14680  comments=1
对象:6467 个 / 994.201 MiB,两端逐字节相等
```

`nonempty` 是最严的一条(`length(state)>0 AND content_text<>''`):一次产出空 `state`
的恢复能通过所有「表在不在」式断言,但过不了它。

然后**用浏览器真的看一眼**(唯一替代不了的一步):

- [x] 能用生产的账号密码登录(证明 users 表回来了)
- [x] 侧栏工作区数量与生产一致
- [x] 打开一篇**带图片**的页面,图能显示(证明第 7 步有效)
- [x] 搜索一个词,结果与生产一致
- [x] 打开一篇页面,历史/评论还在(证明 CRDT 与关联表回来了)

> ⚠️ **判据要写对**。第一版把第二条写成「侧栏工作区数量 = 32」,而 32 是
> `workspaces` 的**表行数**;侧栏列的是**当前登录用户的成员关系**,是 30。
> 于是一次完美的恢复被报成了偏差,还顺带引出一个不存在的解释(「备份时多了」)。
> **判据来自应用的语义,不是表的行数** —— 拿错了就会把正确当成错误,而这比反过来更贵:
> 它会让人去"修"一个没坏的东西。核对方法:`workspace_members` 按用户分组,两端对比。

**耗时**:程序化对比秒级;浏览器核验 ~5 分钟。

## 走完之后

**累计耗时 ≈ 10 分钟**(机器侧 ~7 分钟 + 浏览器核验 ~5 分钟)。已填进
`dr-plan.md` §3,该节的「未实测」已删。

**但当天的墙钟是数小时** —— 差额不是浪费,是**走的过程中撞上并修掉了四个会让流程停死的
问题**。这四条才是这次演练真正的产出;10 分钟这个数字,是修完之后才成立的:

| # | 撞上的 | 修法 |
| --- | --- | --- |
| 1 | 阿里云余额 81.93,按量付费要 ≥100,**开不出机器** | 进 `dr-plan §9` 前置条件清单,配检查命令 |
| 2 | 新机器**连不上 Docker Hub**,postgres/rustfs/traefik 一个都拉不到 —— 「给台新机器就能重建」这个前提是假的 | `provision.yml` 配 pull-through 代理并**显式** pull,失败硬停 |
| 3 | **凭据哪儿都没备**,重建走到"栈起来了、是空的"就停死 | v0.13.40 `_config` leg + v0.13.41 `restore-config` |
| 4 | 起栈时 api 跑迁移把 schema 建满,**`pg_dump` 灌不进去** | 顺序改成「凭据 → 空库 → 灌 → 起 api」 |

这四条的共同点:**每一条在文档里都是"假设成立",而且看起来毫无问题。**

**下一次演练要做的**(这份清单本身就是这次的产出):

1. **仍然按这份走,并且预期会撞上新的东西。** 演练的价值不是确认它能用,是找出它哪里
   不能用 —— 这次找出四个,下次大概率不是零。
2. **先查 `dr-plan §9` 的前置条件**,一条命令一条命令地查。这次就是被其中一条(余额)
   当场挡下的,而它在那之前看起来毫无问题。
3. `cd dr/aliyun && tofu destroy`,**当天做**。DNS 记录也归 tofu(`dns.tf`),会一并
   收走 —— 不需要再手工去删 A 记录。公网 IP 随实例分配、不是独立 EIP,同样一并收走。
4. 🔴 **第 4 步之后,演练机持有生产凭据。** 当天销毁是硬要求,不是卫生习惯。
   真过夜了,正确处置不是"应该没事",是**轮换那批凭据**。

## 已知会卡住的地方(这一栏现在是实测,不是预测)

| 地方 | 现象 | 处置 |
| --- | --- | --- |
| 第 0 步 | `InvalidAccountStatus.NotEnoughBalance` | 余额 ≥100 元。网络层已建的部分免费,充值后再 `apply` 即可续上 |
| 第 0 步 | 实例创建完成后 ssh 报 `Connection closed` | **不是故障**,sshd 还要 ~20 秒。别据此去查安全组 |
| 第 2 步 | `curl get.docker.com \| sh` 连接重置 | 国内 ECS 到 `download.docker.com` 不通,走阿里云 docker-ce 镜像 |
| 第 2 步 | 三个第三方镜像一个都拉不到 | `registry-1.docker.io` 完全不可达,`provision.yml` 已配 pull-through 代理 |
| 第 3 步 | traefik 面板凭据"莫名其妙不对" | `.env` 里 htpasswd 哈希的 `$` 被 compose 当变量吃了,模板里已 `$` → `$$` |
| 第 4 步 | `error: OSS_BUCKET is required for the backup` | 听起来像凭据缺失,实际是 `OSS_BUCKET` 在 `.env` 而不是 `.env.secrets`。两个文件都要挂 |
| 第 5→6 | `ERROR: type "object_type" already exists` | 起栈时 api 跑了迁移,库不空。**顺序问题**:凭据 → 空库 → 灌 → 起 api |
| 第 6 步 | 🔴 想 `./dc --profile backup up -d backup` | **绝对不行**:daemon run-on-start 会往生产仓库写并 `forget --prune`。用一次性容器 |
| 第 6 步 | `tail` 看快照 → 拿到旧的那组 | 按 hostname 分组,要 `grep` 出所有行 |
| 第 7 步 | rclone 403 | 看清报错里是**源**(`rustfs:`,本机凭据)还是**目的**(`ossblob:`,OSS 凭据) |
| 第 7 步 | 容器里解析不到 `rustfs` | 要 `--network mica_default` |
| 第 8 步 | 侧栏数量"对不上" | 判据别用表行数。侧栏列的是**登录用户的成员关系**,查 `workspace_members` 按用户分组 |
| 全程 | ssh 太密被上游限流 | 一次 ssh 里用 heredoc 跑完一组命令(`dr-plan` §10) |
