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

variable "public_key" {
  description = <<-EOT
    公钥**内容**(`ssh-ed25519 AAAA... comment`)。留空则去读 `public_key_path`。

    **CI 用的是这个,不是路径** —— runner 上没有 `~/.ssh/`。Action 里:
    `TF_VAR_public_key: $${{ secrets.DR_SSH_PUBLIC_KEY }}`(HCL 里 `$$` 转义 `$`)。

    公钥本身不是秘密(它就是拿来公开的),放 repository **variable** 也行;
    放 secret 只是省得每次确认它到底该不该公开。

    ⚠️ **真正要命的是私钥**:Action 建出机器之后,能登进去的只有持有私钥的人。
    所以这把私钥和 `MICA_BACKUP_PASSWORD` 一样,归宿是**密码管理器** —— 只存在
    某台笔记本上的私钥,在那台笔记本也没了的时候等于没有。
  EOT
  type        = string
  default     = ""
}

variable "public_key_path" {
  description = <<-EOT
    公钥文件路径。**必须在创建实例时注入** —— 这是整个流程里最容易漏、且事后补不了
    的一步:机器起来你进不去,ansible 也连不上。

    只在 `public_key` 留空时才读。没有密钥就先建一把:
    `ssh-keygen -t ed25519 -C "mica-dr"`(一路回车)。
  EOT
  type        = string
  default     = "~/.ssh/id_ed25519.pub"
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

variable "root_password" {
  description = <<-EOT
    root 密码。留空(默认)= 不设,只能用密钥登录。

    **为什么值得设**:ssh 进不去的时候还剩一条路 —— 阿里云控制台的 VNC。而容灾场景里
    「ssh 进不去」恰恰是常态:安全组填错、网络没通、sshd 没起来。密钥登录救不了这些,
    因为它们都发生在 ssh 之前。

    **代价,必须知道**:它会**明文写进 state 文件**。所以要用就把 state 加密打开 ——
    见 README「设 root 密码」。这正是选 OpenTofu 而不是 Terraform 的那条理由所指的
    场景(dr-plan §7.2.1),现在轮到它兑现了。

    阿里云的要求:8-30 位,大写/小写/数字/特殊字符里至少占三类。
  EOT
  type        = string
  default     = ""
  sensitive   = true

  validation {
    condition     = var.root_password == "" || length(var.root_password) >= 8
    error_message = "阿里云要求 root 密码至少 8 位(8-30,且含大小写/数字/特殊字符中的三类)。"
  }
}

variable "instance_type_family" {
  description = <<-EOT
    实例规格族,留空 = 不限(推荐)。

    曾经写死 `ecs.e`(经济型),而实测 cn-shenzhen-b 里符合 2C4G 的 ecs.e 一个都没有,
    plan 直接失败。家族的可售性按地域/可用区变 —— 钉死它等于给自己加一个会在容灾
    当天失效的前提。
  EOT
  type        = string
  default     = ""
}

variable "dns_domain" {
  description = <<-EOT
    主域名(不带子域)。演练的两条 A 记录建在它下面,`destroy` 时一起删。

    只有域名托管在**阿里云 DNS** 时这份配置才管得着 —— 换 DNS 服务商就把
    `dns.tf` 换掉,机器那部分不受影响。
  EOT
  type        = string
  default     = "cloudcele.com"
}

variable "dns_rr" {
  description = <<-EOT
    应用的子域前缀,最终是 `<dns_rr>.<dns_domain>`。

    ⚠️ **绝不要填生产的 `mica`**:这条记录归 tofu 管,`destroy` 会删掉它。
    真恢复时改生产解析是控制台上的手工动作,故意不在这份配置里。
  EOT
  type        = string
  default     = "mica-dr"
}

variable "dns_rr_s3" {
  description = "对象存储的子域前缀。浏览器直接 presign 到它,所以必须是独立可解析的名字。"
  type        = string
  default     = "mica-s3.dr"
}

variable "dns_ttl" {
  description = <<-EOT
    秒。阿里云**免费版解析的下限是 600** —— 填更小会被 API 拒绝。

    这不只是演练的细节:它意味着「改 DNS 切到新机器」这一步天然带着**最多 10 分钟**
    的传播尾巴,得算进 RTO。要更快就得付费版(支持到 1 秒),或者不靠 DNS 切换
    (浮动 IP / 负载均衡)。
  EOT
  type        = number
  default     = 600
}
