# 容灾恢复演练 —— 手工走一遍(工作表)

> **这是一份要被你改写的文档。** 每一步都标着 `[推演]`,意思是**写下来的时候没人走过**。
> 走通一步就改成 `[已验证]` 并填耗时;卡住就把真实报错抄进「实际」栏。
> 走完之后,这份文档才第一次配得上叫 runbook。
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

| 需要 | 从哪来 | 缺了会怎样 |
| --- | --- | --- |
| 测试域名两条 A 记录 | 你的 DNS | ACME 签不出证书 |
| `RUSTIC_PASSWORD` | 密码管理器(节点 `.env.secrets` 里叫 **`MICA_BACKUP_PASSWORD`**) | **拿到一个完好但永远打不开的仓库** |
| OSS 读凭据 | 密码管理器 | 读不到备份 |
| `.env` / `.env.secrets` 全套键 | 密码管理器 | **它们不在任何备份里**(`dr-plan` §2.2) |
| 阿里云可用额度 **≥ 100 元** | 充值 | **开不出按量付费实例**——实测拦在这里(见步骤 0) |

`.env.secrets` 的键名(值不在这里):

```
JWT_SECRET  POSTGRES_PASSWORD  S3_ACCESS_KEY  S3_SECRET_KEY
MICA_BACKUP_TOKEN  MICA_BACKUP_PASSWORD  OSS_ACCESS_KEY_ID  OSS_SECRET_ACCESS_KEY
MICA_MAIL_ACCESS_KEY_ID  MICA_MAIL_ACCESS_KEY_SECRET  MICA_BACKUP_PGURL
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
WSL 发行版,所以借容器当控制端 —— Docker Desktop 已经在跑:

```bash
docker run --rm -v "C:/data/codes/mica-will-laptop:/work" -v "C:/Users/willz/.ssh:/ssh:ro" -v "<scratch>/run-ansible.sh:/run.sh:ro" -e DR_HOST=<IP> -e TRAEFIK_BASIC_AUTH="admin:<hash>" --entrypoint sh alpine/ansible /run.sh ansible/provision.yml -e target=mica-dr
```

`run-ansible.sh` 做三件事:把公钥复制出来 `chmod 600`(Windows 挂进来的权限是 0777,
ssh 会拒绝用)、装 `ansible/requirements.yml` 里的 collection、跑 playbook。

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

### 4. `[推演]` 放配置与凭据

```bash
cd /data/mica
# docker-compose.yaml 从 tag 取,和生产同一份:
git show v0.13.39:deploy/docker-compose.yml > docker-compose.yaml
# .env(非机密)与 .env.secrets(密码管理器)手工写
chmod 600 .env.secrets
```

**判据**:`.env.secrets` 的键都在,且 `RUSTFS_S3_*` 与 `S3_*` 一致(或干脆不写)。
**实际**:____  **耗时**:____

### 5. `[推演]` 起空栈

```bash
# ./dc 是 ansible 生成的包装器,新机器上没有 —— 手工等价物:
alias dc='docker compose --env-file /data/mica/.env --env-file /data/mica/.env.secrets'
dc up -d
```

> **不要用裸 `docker compose`。** 它只自动读 `.env`,而凭据在 `.env.secrets`;
> compose 里写的是 `${VAR:-默认}` 不是 `${VAR:?}`,所以缺了**不报错**,而是拿默认值
> 把库 initdb 出来 —— 一颗要到下次部署才炸的雷(`upgrade-infra.md` 有完整故事)。

**判据**:`dc ps` 全部 running;`curl -s localhost:8080/api/health` 报对版本。
**实际**:____  **耗时**:____

### 6. `[推演]` 灌数据库

```bash
dc exec backup rustic snapshots --filter-label _pgdump | grep -E "^\| [0-9a-f]{8}"
```

> **别 `tail` 取最新** —— 输出按 hostname 分组(容器 id 每次部署都变),`tail` 只会
> 给你最后一组。要 grep 出所有行再看日期。2026-08-29 我因此连续两次报错了中断时长
> (`dr-plan` §8.2)。

```bash
dc exec backup rustic restore latest /tmp/pg --filter-label _pgdump
dc exec backup sh -c 'head -3 /tmp/pg/mica.sql; grep -c "^COPY public\." /tmp/pg/mica.sql'
dc exec backup cat /tmp/pg/mica.sql > /tmp/mica.sql
dc exec -T postgres psql -U mica -d mica -v ON_ERROR_STOP=1 < /tmp/mica.sql
shred -u /tmp/mica.sql     # 明文全库,含口令 hash —— 用完即毁
```

**判据**:`COPY public.` 段数 ≥ 21;导入无 ERROR。
**实际**:____  **耗时**:____

### 7. `[推演]` 灌对象字节(图片)

```bash
dc exec backup rclone --config /etc/rclone/rclone.conf \
  copy ossblob:mica-backup-cloudcele/mica-blobs rustfs:mica --transfers 4 --stats 30s
```

**判据**:传输完成、无 403。
**实际**:____  **耗时**:____

> 403 打在**源**还是**目的**,含义完全不同:源(`rustfs:`) = 本地那对凭据不对;
> 目的(`ossblob:`) = OSS 凭据不对。看清报错里的 bucket 名再动手。

### 8. `[推演]` 核验(版本号证明不了数据回来了)

```bash
curl -s https://mica-dr.cloudcele.com/api/health     # 版本
curl -s https://mica-dr.cloudcele.com/api/ready      # 就绪
```

然后**用浏览器真的看一眼**:

- [ ] 能用生产的账号密码登录(证明 users 表回来了)
- [ ] 侧栏工作区数量与生产一致
- [ ] 打开一篇**带图片**的页面,图能显示(证明第 7 步有效)
- [ ] 搜索一个词,结果与生产一致(证明正文与索引都在)
- [ ] 打开一篇页面,历史/评论还在(证明 CRDT 与关联表回来了)

**实际**:____  **耗时**:____

---

## 走完之后

**累计耗时 = ____** ← 填进 `dr-plan.md` §3 的 RTO 那格,并把该节的「未实测」改掉。
这是这次演练唯一的、也是最重要的产出。

然后:

1. 把每一步的 `[推演]` 改成 `[已验证]`,或写下它实际是怎么失败的。
2. **卡住的地方就是自动化的清单** —— 那些差异才是 OpenTofu / Ansible 真正要处理的
   东西,而不是把这份文档照着翻译一遍。
3. `cd dr/aliyun && tofu destroy`,**当天做** —— 失败的演练最容易留下还在计费的资源。
   (公网 IP 随实例分配、不是独立 EIP,所以 destroy 会一并收走。)
4. 删掉测试域名的 A 记录。

## 已知会卡住的地方(先说,免得你以为是自己弄错了)

| 地方 | 现象 | 为什么 |
| --- | --- | --- |
| 第 2 步 | 全靠手打 | provisioning 层不存在,这正是演练要量化的缺口 |
| 第 3 步 | 忘了建网络,mica 栈起不来 | compose 声明 `external: true` |
| 第 5 步 | 用裸 `docker compose` → 库用默认口令建出来 | 生产踩过一次 |
| 第 6 步 | `tail` 看快照 → 拿到旧的那组 | 按 hostname 分组 |
| 第 7 步 | rclone 403 | 两对凭据是否一致 |
| 全程 | ssh 太密被上游限流 | 一次 ssh 里用 heredoc 跑完一组命令(`dr-plan` §9) |
