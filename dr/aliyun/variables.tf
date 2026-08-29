variable "region" {
  description = "必须与备份桶同地域 —— 跨地域拉 1.5 GB 又慢又要流量费。"
  type        = string
  default     = "cn-shenzhen"
}

variable "name" {
  description = "所有资源的名字前缀。演练与真恢复用不同的值,免得 destroy 错对象。"
  type        = string
  default     = "mica-dr-drill"
}

variable "instance_type" {
  description = <<-EOT
    留空则按 cpu/memory 自动挑一个当前可用的规格。
    写死型号的风险是它在某个可用区不可售,而报错要到 apply 才出现。
  EOT
  type        = string
  default     = ""
}

variable "cpu_core_count" {
  description = "生产实测 2 核。"
  type        = number
  default     = 2
}

variable "memory_size" {
  description = "GiB。生产实测 3.5,这里取 4(没有 3.5 的规格档)。"
  type        = number
  default     = 4
}

variable "image_name_regex" {
  description = <<-EOT
    镜像名的正则,交给 `alicloud_images` 现查 —— 不写死 image id:同一个版本在不同
    地域是不同的 id,而写死的那份要到 apply 才报错。

    阿里云公共镜像的命名形如 `ubuntu_26_04_x64_20G_alibase_20260401.vhd`,
    所以按前缀匹配即可。想换版本就改这里:`^ubuntu_24_04_x64.*`。
  EOT
  type        = string
  default     = "^ubuntu_26_04_x64.*"
}

variable "system_disk_size" {
  description = <<-EOT
    GiB。40 是量出来的不是拍的:docker 镜像 ~4G + 恢复的数据 ~1.5G
    (postgres 卷 439MB + rustfs 卷 1.01GB) + 中途那份 634MB 明文 dump。
    生产 /data 是 100G,但那 69G 大半不是 mica 的。
  EOT
  type        = number
  default     = 40
}

variable "public_key_path" {
  description = <<-EOT
    公钥文件路径。**必须在创建实例时注入** —— 这是整个流程里最容易漏、且事后补不了
    的一步:机器起来你进不去,ansible 也连不上。
  EOT
  type        = string
  default     = "~/.ssh/id_rsa.pub"
}

variable "ssh_cidr" {
  description = <<-EOT
    允许 ssh 的来源。默认全开是为了演练顺手,**真恢复时收窄到你的出口 IP** ——
    这台机器上会有一份完整的生产数据。
  EOT
  type        = string
  default     = "0.0.0.0/0"
}

variable "bandwidth_mbps" {
  description = "公网出带宽峰值。要从 OSS 拉 1.5 GB,太小就是干等。"
  type        = number
  default     = 10
}

variable "profile" {
  description = <<-EOT
    共享凭据文件里的 profile 名。留空 = 用默认的那个。
    给 DR 单开一个 profile 的好处:那把 key 可以只有 ECS + VPC 权限,
    而日常那把不必因此降权。
  EOT
  type        = string
  default     = ""
}

variable "shared_credentials_file" {
  description = "凭据文件路径。留空 = provider 的默认(~/.aliyun/config.json)。"
  type        = string
  default     = ""
}
