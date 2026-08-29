# ── 先问云:哪个可用区、哪个规格、哪个镜像现在真的能买到 ────────────────────────
#
# 全部走 data source 而不是写死 id:型号与镜像 id 会随时间和可用区变,写死的那份
# 在 apply 那一刻才报错 —— 而容灾时你没有调试的余裕。

data "alicloud_zones" "available" {
  available_resource_creation = "Instance"
  available_disk_category     = "cloud_essd"
}

data "alicloud_instance_types" "matched" {
  availability_zone = data.alicloud_zones.available.zones[0].id
  cpu_core_count    = var.cpu_core_count
  memory_size       = var.memory_size
  # 默认不限家族:写死 "ecs.e" 时实测在 cn-shenzhen-b 一个都匹配不到(2026-08-29
  # 首次 plan)。家族的可售性按地域/可用区变,钉死它等于给自己加一个会在容灾当天
  # 失效的前提。
  instance_type_family = var.instance_type_family != "" ? var.instance_type_family : null
}

data "alicloud_images" "ubuntu" {
  owners      = "system"
  name_regex  = var.image_name_regex
  most_recent = true
}

locals {
  zone_id = data.alicloud_zones.available.zones[0].id
  # try 而不是直接下标:匹配不到时给 null,让下面的 precondition 说出人话,
  # 而不是抛一个 "index 0 out of range" 让你去猜是镜像没了还是地域不对。
  image_id = try(data.alicloud_images.ubuntu.images[0].id, null)
  # 显式指定优先;否则取自动匹配到的第一个。
  # try 而不是 coalesce:coalesce 在参数全空时**自己抛错**,发生在 locals 求值阶段,
  # 于是下面那条 precondition 根本轮不到 —— 实测首次 plan 拿到的就是 "Call to
  # function coalesce failed",既没说哪个地域没货,也没说该怎么办。
  instance_type = var.instance_type != "" ? var.instance_type : try(
    data.alicloud_instance_types.matched.instance_types[0].id,
    null,
  )
}

# ── 网络 ──────────────────────────────────────────────────────────────────────

resource "alicloud_vpc" "dr" {
  vpc_name   = var.name
  cidr_block = "172.24.0.0/16"
}

resource "alicloud_vswitch" "dr" {
  vswitch_name = var.name
  vpc_id       = alicloud_vpc.dr.id
  cidr_block   = "172.24.1.0/24"
  zone_id      = local.zone_id
}

resource "alicloud_security_group" "dr" {
  security_group_name = var.name
  vpc_id              = alicloud_vpc.dr.id
  description         = "mica DR drill — ssh + http + https"
}

# 22 收窄到 var.ssh_cidr;80/443 必须对全网开 —— ACME 的 TLS-ALPN-01 挑战由
# Let's Encrypt 从它自己的出口打进来,你不知道那是哪个 IP。少开 443 就签不出证书,
# 而它不会响亮地失败,只会安静地重试到用完额度。
resource "alicloud_security_group_rule" "ssh" {
  type              = "ingress"
  ip_protocol       = "tcp"
  port_range        = "22/22"
  security_group_id = alicloud_security_group.dr.id
  cidr_ip           = var.ssh_cidr
}

resource "alicloud_security_group_rule" "http" {
  type              = "ingress"
  ip_protocol       = "tcp"
  port_range        = "80/80"
  security_group_id = alicloud_security_group.dr.id
  cidr_ip           = "0.0.0.0/0"
}

resource "alicloud_security_group_rule" "https" {
  type              = "ingress"
  ip_protocol       = "tcp"
  port_range        = "443/443"
  security_group_id = alicloud_security_group.dr.id
  cidr_ip           = "0.0.0.0/0"
}

# ── 密钥对:声明式地解掉「先有鸡还是先有蛋」 ──────────────────────────────────

resource "alicloud_ecs_key_pair" "dr" {
  key_pair_name = var.name
  public_key    = trimspace(file(pathexpand(var.public_key_path)))
}

# ── 机器 ──────────────────────────────────────────────────────────────────────

resource "alicloud_instance" "dr" {
  instance_name   = var.name
  host_name       = var.name
  image_id        = local.image_id
  instance_type   = local.instance_type
  security_groups = [alicloud_security_group.dr.id]
  vswitch_id      = alicloud_vswitch.dr.id
  key_name        = alicloud_ecs_key_pair.dr.key_pair_name

  # 留空则整个不传,实例保持「仅密钥登录」。设了就明文进 state —— 见变量说明。
  password = var.root_password != "" ? var.root_password : null

  system_disk_category = "cloud_essd"
  system_disk_size     = var.system_disk_size

  # 按量付费 + 按流量计费的公网 IP:演练完 `tofu destroy` 一起收走。
  # `internet_max_bandwidth_out > 0` 本身就会分配公网 IP,不需要单独的 EIP 资源 ——
  # 独立 EIP 正是「只删实例、IP 还在计费」那种尾巴的来源。
  instance_charge_type       = "PostPaid"
  internet_charge_type       = "PayByTraffic"
  internet_max_bandwidth_out = var.bandwidth_mbps

  tags = {
    purpose   = "mica-dr-drill"
    ephemeral = "true" # 看到这个标签的机器都可以删
  }

  lifecycle {
    # 在 plan 阶段就说清楚是哪一步没成。没有这条,匹配不到规格时 instance_type
    # 会是 null,报错要到 apply 才出现,而且指向的是 API 而不是原因。
    precondition {
      condition     = local.instance_type != null && local.instance_type != ""
      error_message = "在 ${var.region} 没匹配到 ${var.cpu_core_count} 核 / ${var.memory_size} GiB 的 ecs.e 规格。显式设 instance_type,或放宽 cpu_core_count / memory_size。"
    }

    precondition {
      condition     = local.image_id != null
      error_message = "在 ${var.region} 没有匹配 ${var.image_name_regex} 的公共镜像。用 `aliyun ecs DescribeImages --RegionId ${var.region} --ImageOwnerAlias system` 看看有哪些,再改 image_name_regex(例如换成 ^ubuntu_24_04_x64.*)。"
    }
  }
}
