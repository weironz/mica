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

**环境变量,本地和 CI 用同一套机制。**

```powershell
# Windows,设一次,新开的终端才生效
setx ALICLOUD_ACCESS_KEY "..."
setx ALICLOUD_SECRET_KEY "..."
```

```bash
# Linux/macOS 或临时用
export ALICLOUD_ACCESS_KEY=... ALICLOUD_SECRET_KEY=...
```

**为什么不是凭据文件**(`~/.aliyun/config.json` + `profile`,provider 也支持):
CI 里没法干净地放一个文件,所以 Action 必然用环境变量 —— 本地再用文件,就是**同一个
事实两套机制**。今天已经为这个形状付过两次代价(`MICA_TOKEN`/`MICA_PAT`、
`S3_*`/`RUSTFS_S3_*`,见 `dr-plan` §8),不必再埋一颗。

`profile` / `shared_credentials_file` 两个变量仍然留着 —— 你若已经有 aliyun CLI 的
profile,填上就能用。但**默认路径是环境变量**。

**不要**写进 `.tf`(会被提交),**也不要**写进 `.tfvars` —— 它虽在 `.gitignore` 里,
但就躺在仓库目录下,一次 `git add -f` 或换机复制目录就泄了。**别指望 gitignore 保护
密钥**。

**给 DR 单开一个 RAM 用户**,权限只给 **ECS + VPC**:改 DNS 是手工的,不需要 DNS 权限;
读备份是在**新机器上**用 `.env.secrets` 里那对 OSS key,和这把无关。这样万一它泄了,
最大损害是有人在你账号里开关机器,而不是读走全部数据。

### 将来的 GitHub Action 长这样(现在写清楚,到时照抄)

```yaml
      - uses: opentofu/setup-opentofu@v1
      - run: tofu init && tofu apply -auto-approve
        working-directory: dr/aliyun
        env:
          ALICLOUD_ACCESS_KEY: ${{ secrets.ALICLOUD_ACCESS_KEY }}
          ALICLOUD_SECRET_KEY: ${{ secrets.ALICLOUD_SECRET_KEY }}
          # state 必须放对象存储(runner 是一次性的),并开加密:
          TF_ENCRYPTION: ${{ secrets.TF_ENCRYPTION }}
```

**不需要装 aliyun CLI** —— provider 直接读这两个环境变量。CLI 只是临时查东西时顺手
(列镜像、查规格),不在关键路径上。

## 设 root 密码(可选,但建议)

```hcl
# terraform.tfvars —— 注意下面那段警告,别就这么放着
root_password = "改成你自己的"
```

**为什么值得设**:ssh 进不去时还剩一条路 —— 控制台 VNC。而容灾场景里「ssh 进不去」
恰恰是常态:安全组填错、网络没通、sshd 没起来。**密钥登录救不了这些,因为它们都发生在
ssh 之前。**

**代价**:密码会**明文写进 state 文件**。所以设了密码就要把 state 加密打开 ——
这正是选 OpenTofu 而不是 Terraform 的那条理由(`dr-plan` §7.2.1),现在轮到它兑现。

OpenTofu 支持用 `TF_ENCRYPTION` 环境变量配置加密(**已实测**:给它一段非法 HCL,
`tofu show` 会当场报解析错,证明它确实被读取),所以不必改任何 `.tf`:

```bash
export TF_ENCRYPTION='
key_provider "pbkdf2" "main" {
  passphrase = "一段足够长的口令,放进你的密码管理器"
}
method "aes_gcm" "main" {
  keys = key_provider.pbkdf2.main
}
state {
  method = method.aes_gcm.main
}
'
tofu apply
```

⚠️ **口令丢了 = state 打不开 = tofu 再也管不了这些资源**(机器还在跑,但你只能去控制台
手动删)。所以它和 `MICA_BACKUP_PASSWORD` 一样,归宿是密码管理器,不是这台机器。

不想折腾加密就**别设密码** —— 只用密钥登录,state 里就没有任何秘密。演练机尤其可以
这样:反正它当天就删。

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
| `image_name_regex` | `^ubuntu_26_04_x64.*` | 26.04 LTS。改这里换版本(如 `^ubuntu_24_04_x64.*`);**不写死 image id** —— 同一版本在不同地域是不同的 id |
| `system_disk_size` | 40 | docker 镜像 ~4G + 恢复的数据 ~1.5G(postgres 439MB + rustfs 1.01GB)+ 中途那份 634MB 明文 dump |
| `root_password` | 空 | 不设 = 仅密钥登录,state 里没有秘密。设了要连带打开 state 加密 |
| `ssh_cidr` | `0.0.0.0/0` | 演练顺手。**真恢复时收窄到你的出口 IP** —— 那台机器上会有一份完整的生产数据 |

规格、镜像、可用区都走 data source 现查,不写死 id:型号会随时间和可用区变,写死的
那份在 apply 那一刻才报错,而容灾时没有调试的余裕。匹配不到会在 **plan** 阶段就说明
原因(`precondition`),不会拿一个空值去 apply。

## 已验证到哪一步

`tofu init` / `validate` / `fmt` 在本机跑过(2026-08-29,OpenTofu 1.12.5,
provider 1.240+)。**`plan` 与 `apply` 没跑过** —— 它们要真凭据,而那是你执行演练
那一刻的事。schema 与真实可用性以 `tofu plan` 为准。
