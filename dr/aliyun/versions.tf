# 演练/容灾用的机器,只到「一台能 ssh 进去的机器」为止。
#
# 边界(docs/dr-plan.md §7.2):再往后一步都不许知道自己在哪家云上 —— 装 Docker、
# 起 Traefik、灌数据全是与云无关的,那才是跨云能力真正的来源。想换腾讯云/GCP,
# 复制这个目录改 provider 与 resource,后面的 runbook 一个字都不用动。

terraform {
  required_version = ">= 1.6"

  required_providers {
    alicloud = {
      source  = "aliyun/alicloud"
      version = "~> 1.240"
    }
  }

  # state 在 OSS,不在本地。这不是洁癖,是 CI 的硬要求:**GitHub runner 是一次性的**,
  # 本地 state 随 job 一起消失,于是它刚建出来的机器变成 tofu 再也管不到的孤儿 ——
  # 不会报错,只是一直计费,直到有人翻控制台才发现。
  #
  # 桶用现成的备份桶加一个自己的前缀。与备份互不重叠:rustic 在 OSS_ROOT=mica 下,
  # 对象镜像在 mica-blobs/ 下,谁也 prune 不到 tofu/。
  #
  # 凭据走 ALICLOUD_ACCESS_KEY / ALICLOUD_SECRET_KEY,与 provider 同一对。
  backend "oss" {
    bucket = "mica-backup-cloudcele"
    prefix = "tofu/dr"
    region = "cn-shenzhen"
  }

  # state 加密**故意没开**。这份 state 里只有实例 id / 公网 IP / 安全组 id / 公钥 ——
  # 没有一样是秘密。开了就多一个必须被携带的口令,而「要带在外面的秘密」这份清单
  # (dr-plan §2.2)正在被努力压短,不该为了看起来周全而加长它。
  #
  # ⚠️ **设了 var.root_password 就必须打开**:那个值会明文进 state。这也正是选
  # OpenTofu 而不是 Terraform 的那条决定性理由(dr-plan §7.2.1)兑现的地方:
  #
  #   encryption {
  #     key_provider "pbkdf2" "main" { passphrase = ... }  # 或走 TF_ENCRYPTION 环境变量
  #     method "aes_gcm" "main" { keys = key_provider.pbkdf2.main }
  #     state { method = method.aes_gcm.main }
  #   }
}

provider "alicloud" {
  region = var.region

  # 凭据**永远不写在这里**。三种来源,provider 自己会找,按下面的顺序:
  #
  #   1. 环境变量 ALICLOUD_ACCESS_KEY / ALICLOUD_SECRET_KEY
  #   2. 共享凭据文件(默认 ~/.aliyun/config.json,由 `aliyun configure` 生成)
  #   3. ECS 实例 RAM 角色(在阿里云机器上跑时,连 key 都不需要)
  #
  # `profile` 留空就用文件里的默认 profile;想区分「日常」和「DR 专用」两套 key
  # 时才填。见 README「凭据放哪」。
  profile                 = var.profile
  shared_credentials_file = var.shared_credentials_file
}
