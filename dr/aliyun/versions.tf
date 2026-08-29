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

  # state 放本地即可:这次演练的 state 里只有实例 id / IP / 安全组 id,没有凭据。
  #
  # 真容灾时要挪到对象存储 —— 绝不能放在那台会挂的机器上。同时打开 OpenTofu 的
  # state 加密(这是选它而非 Terraform 的决定性理由,见 dr-plan §7.2.1):
  #
  #   backend "oss" {
  #     bucket = "mica-backup-cloudcele"
  #     prefix = "tofu/dr"
  #     region = "cn-shenzhen"
  #   }
  #
  # encryption {
  #   key_provider "pbkdf2" "main" { passphrase = var.state_passphrase }
  #   method "aes_gcm" "main" { keys = key_provider.pbkdf2.main }
  #   state { method = method.aes_gcm.main }
  # }
}

provider "alicloud" {
  region = var.region
  # 凭据只从环境变量来,永远不进文件:
  #   export ALICLOUD_ACCESS_KEY=... ALICLOUD_SECRET_KEY=...
}
