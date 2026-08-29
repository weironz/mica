# ── DNS:和机器同生共死 ────────────────────────────────────────────────────────
#
# 放进 tofu 而不是手点控制台,图的是 `destroy` 那一下:记录跟着机器一起消失,
# 不会留下一条指向已删实例的野记录 —— 那种记录不报错,只是安静地指向别人的 IP。
#
# ⚠️ **绝不要把 dns_rr 指成生产的 `mica`**。这些记录归 tofu 管,意味着
# `tofu destroy` 会**删掉**它们。真恢复时改生产解析是控制台上的一次手工操作,
# 故意不放进这份配置。

resource "alicloud_alidns_record" "app" {
  domain_name = var.dns_domain
  rr          = var.dns_rr
  type        = "A"
  value       = alicloud_instance.dr.public_ip
  ttl         = var.dns_ttl
}

resource "alicloud_alidns_record" "s3" {
  domain_name = var.dns_domain
  rr          = var.dns_rr_s3
  type        = "A"
  value       = alicloud_instance.dr.public_ip
  ttl         = var.dns_ttl
}

# Traefik 的面板路由在 compose 里是写死存在的(那份 compose 是从生产原样抓下来的,
# 不该为演练改它)。名字解析不到的话,traefik 会为它反复向 ACME 要证书并反复失败 ——
# 不影响 mica-dr 的证书,但会把日志填满、也白耗签发尝试。给它一条记录最省事。
resource "alicloud_alidns_record" "traefik" {
  domain_name = var.dns_domain
  rr          = "traefik.${var.dns_rr}"
  type        = "A"
  value       = alicloud_instance.dr.public_ip
  ttl         = var.dns_ttl
}
