output "public_ip" {
  description = "填进 docs/dr-drill.md 第 0 步,并把 DNS A 记录指向它。"
  value       = alicloud_instance.dr.public_ip
}

output "ssh" {
  description = "直接复制去连。"
  value       = "ssh root@${alicloud_instance.dr.public_ip}"
}

output "chosen" {
  description = "云实际给了什么 —— 抄进 dr-drill.md 第 0 步的「实际」栏。"
  value = {
    zone          = local.zone_id
    instance_type = local.instance_type
    image         = data.alicloud_images.ubuntu.images[0].id
    disk_gib      = var.system_disk_size
  }
}
