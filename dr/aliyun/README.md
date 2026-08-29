# 容灾/演练机器(阿里云)

**职责到「一台能 ssh 进去的机器」为止。** 装 Docker、起 Traefik、灌数据全部与云无关 ——
那才是跨云能力真正的来源(`docs/dr-plan.md` §7.2)。要换腾讯云/GCP:复制本目录、
换 provider 与 resource,`docs/dr-drill.md` 第 1 步之后一个字都不用改。

## 为什么是 OpenTofu 不是 Terraform

只有一条是决定性的:**state 加密**(1.7 起,用你自己的密钥,在写进 backend 之前)。
真容灾时 state 要放对象存储、和备份同一个桶,而只要哪天往变量里塞了口令,Terraform
会明文写进 state。许可证与切换成本都只是加分项 —— 详见 `docs/dr-plan.md` §7.2.1。

## 用之前

```bash
# 1) 装 OpenTofu(Windows)
winget install --id=OpenTofu.Tofu -e

# 2) 凭据 —— 见下一节「凭据放哪」

# 3) 公钥必须在创建实例时注入 —— 事后补不了(进不去机器,ansible 也连不上)
ls ~/.ssh/id_rsa.pub    # 没有就 ssh-keygen -t ed25519
```

## 凭据放哪

**不要写进 `.tf`,也不要写进 `.tfvars`。** 前者会被提交;后者虽然在 `.gitignore` 里,
但它就躺在仓库目录下,一次 `git add -f` 或换台机器复制目录就泄了。

provider 会按这个顺序自己找(已用 `tofu providers schema` 查证):

| 方式 | 怎么做 | 适合 |
| --- | --- | --- |
| **① 共享凭据文件(推荐)** | `aliyun configure --profile mica-dr` | 长期。文件在 `~/.aliyun/config.json`,**在仓库之外**,`aliyun` CLI 与 tofu 共用一份 |
| ② 持久环境变量 | Windows:`setx ALICLOUD_ACCESS_KEY "..."`(新开的终端才生效) | 不想装 aliyun CLI |
| ③ 临时环境变量 | `export ALICLOUD_ACCESS_KEY=...` | 一次性,关掉终端就没了 |

用 ① 时把 profile 名填进 `terraform.tfvars`(**只有名字,没有密钥**):

```hcl
profile = "mica-dr"
```

**给 DR 单开一个 RAM 用户**,权限只给 ECS + VPC —— 改 DNS 那步是手工的,不需要 DNS
权限;日常那把 key 也不必因此降权。

> 将来做成 GitHub Action 时,凭据的归宿是 **repository secrets**,注入成上面②那种
> 环境变量。本地这三种都不会跟着走。

> ④ 还有一种:在阿里云 ECS 上跑 tofu,用实例 RAM 角色(`ecs_role_name`),**一把 key
> 都不需要**。真容灾时如果你手上还有另一台阿里云机器,这是最干净的 —— 但它救不了
> 「整个账号进不去」那种场景,而那正是 §5.3 已拍板不防的。

## 跑

```bash
cd dr/aliyun
tofu init
tofu plan                 # 先看一眼再花钱 —— 这正是选它的理由之一
tofu apply                # 输入 yes
tofu output                # public_ip / ssh / chosen
```

`chosen` 里是云**实际**给的可用区、规格、镜像 —— 抄进 `docs/dr-drill.md` 第 0 步的
「实际」栏,那张表就是下次的输入。

拿到 IP 之后回 `docs/dr-drill.md`,从**第 1 步(DNS)** 继续手工走。

## 用完(当天做)

```bash
tofu destroy
```

公网 IP 是随实例分配的(`internet_max_bandwidth_out > 0`),不是独立 EIP ——
所以 destroy 会一并收走。**独立 EIP 正是「只删实例、IP 还在计费」那种尾巴的来源**,
这里刻意没用它。

## 默认值与它们的来历

| 变量 | 默认 | 为什么 |
| --- | --- | --- |
| `region` | `cn-shenzhen` | 必须与备份桶同地域,跨域拉 1.5 GB 又慢又要流量费 |
| `cpu_core_count` / `memory_size` | 2 / 4 | 生产实测 2 核 3.5 GiB |
| `system_disk_size` | 40 | docker 镜像 ~4G + 恢复的数据 ~1.5G(postgres 439MB + rustfs 1.01GB)+ 中途那份 634MB 明文 dump |
| `ssh_cidr` | `0.0.0.0/0` | 演练顺手。**真恢复时收窄到你的出口 IP** —— 那台机器上会有一份完整的生产数据 |

规格、镜像、可用区都走 data source 现查,不写死 id:型号会随时间和可用区变,写死的
那份在 apply 那一刻才报错,而容灾时没有调试的余裕。匹配不到会在 **plan** 阶段就说明
原因(`precondition`),不会拿一个空值去 apply。

## 已验证到哪一步

`tofu init` / `validate` / `fmt` 在本机跑过(2026-08-29,OpenTofu 1.12.5,
provider 1.240+)。**`plan` 与 `apply` 没跑过** —— 它们要真凭据,而那是你执行演练
那一刻的事。schema 与真实可用性以 `tofu plan` 为准。
