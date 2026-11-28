# Mica 阿里云边缘安全加速（ESA）CDN 接入与验收指南

> 适用范围：通过阿里云 **边缘安全加速 ESA**（基础版 / 标准版，CNAME 接入、全球加速区域）为部署在国内源站的 Mica 纯 Rust WebAssembly 应用提供跨区就近接入、静态资源边缘缓存与加速。

---

## 一、概述与适用场景

Mica 纯 Rust Web 端在生产环境由 [deploy/nginx.conf](../../deploy/nginx.conf) 托管，核心体积集中在经过 Brotli-11 预压缩的主应用 WASM 包（`v0.15.5` 实测 `mica_rust_bg.wasm` 约 `2.51 MB`）与初始化脚本。

当用户从跨地区/海外网络直连国内源站 ECS 时，高 RTT（常见 `200~400 ms`）会导致 TCP/TLS 握手耗时长、单流吞吐受限。接入阿里云 **ESA 全球边缘节点** 后：

- **近端终止 TCP + TLS**：用户就近连入本地边缘 POP 节点，消除跨国握手往返开销；
- **边缘静态缓存 + 专线回源**：`.js`、`.wasm` 静态产物在边缘命中缓存（或在冷命中时通过内部优化链路回源），动态 `/api/*` 与 `/ws/*` 通过 ESA 链路稳定透传回源站。

---

## 二、源站缓存契约与流量边界

在配置 ESA 之前，需先理解源站（Traefik + Nginx + `mica-api-server`）已经定义好的精确缓存契约与流量路径（见 [deploy/nginx.conf](../../deploy/nginx.conf) 与 [crates/api-server/src/routes/files.rs](../../crates/api-server/src/routes/files.rs)）：

| 路径 / 资源匹配 | 源站 `Cache-Control` 行为 | CDN 处理原则 | 说明 |
| --- | --- | --- | --- |
| `/`、`/index.html` | `no-cache`（支持 `ETag` 304） | **禁止强缓存**（`DYNAMIC`） | HTML 入口需随时反映发版后的新 `?v=<hash>` 资源清单 |
| 无 `?v=` 的 `/*.js`、`/*.wasm`、字体 | `no-cache` | **禁止强缓存** | 无内容哈希参数的兜底 URL 必须回源校验 |
| 带哈希参数 `/*.js?v=<sha256>`、`/*.wasm?v=<sha256>` | `public, max-age=31536000, immutable` | **边缘缓存 1 年，保留全部 Query String** | 同时返回 `Content-Encoding: br` 与 `Vary: Accept-Encoding` |
| `/api/*`、`/s/*`、`/reset-password`、`/verify-email`、`/privacy`、`/terms` | 动态响应 | **绕过缓存（Bypass）** | 登录态 Cookie、文档读写、服务端渲染分享页与认证回调 |
| `/ws/*` | WebSocket Upgrade（`101 Switching Protocols`） | **协议升级透传，不缓存** | 多端实时 CRDT 协同与目录变更推送通道 |

### 关于文档图片与附件的特别说明

- 根据 [crates/api-server/src/routes/files.rs](../../crates/api-server/src/routes/files.rs)，编辑器加载图片时通过 `/api/workspaces/{workspace_id}/files/resolve` 获取**带时效签名的对象存储直链**（RustFS 域名）直接下载；
- 复制/导出使用的公开端点 `GET /api/workspaces/{workspace_id}/files/{file_id}/blob[/{filename}]` 也只是返回 **307 临时重定向** 指向最新预签名存储 URL；
- **重要红线**：因此 `/api/*`（包含 `/blob`）**绝对不能在 ESA 中被缓存**，否则签名过期后会导致文档图片加载失败。若后续需要给文档图片做独立 CDN 加速，应针对**对象存储（RustFS）的独立对外域名**单独接入配置，与本主站域名 `<YOUR_DOMAIN>` 分开治理。

---

## 三、ESA 接入配置步骤（CNAME 模式）

以下操作以站点 `<YOUR_DOMAIN>`（例如 `mica.example.com`）、源站公网 IP `<ORIGIN_IP>` 为例：

### 步骤 1：创建站点

1. 进入阿里云 **边缘安全加速 ESA → 站点管理 → 新增站点**。
2. 输入站点域名 `<YOUR_DOMAIN>`。
3. **接入方式** 选择 **`CNAME`**，**加速区域** 选择 **`全球`**，完成站点激活校验直至站点状态变为 **已启用**。

> 注意：站点状态显示「已启用」仅表示控制面审核与套餐绑定完成；在完成 **步骤 2 + 步骤 3** 之前，流量仍直连源站，控制台流量监控会全部为 `0`。

### 步骤 2：在 ESA 控制台添加回源加速记录

进入站点左侧 **DNS → 记录 → 添加记录**，按以下参数填写：

| 字段 | 填写建议 | 注意事项 |
| --- | --- | --- |
| **记录类型** | `A/AAAA` | — |
| **主机记录** | **`@`** | **易错点**：当 ESA 站点本身已是子域名（如 `mica.example.com`）时，控制台输入框右侧后缀已固定为 `.mica.example.com`。此处**必须填 `@`** 代表根记录本身；切勿重复填写 `mica`（否则会变成 `mica.mica.example.com`）。 |
| **代理状态** | **开启（代理加速）** | 保持绿色开启 / 橙色代理图标；关闭则只做纯 DNS 解析、不经过加速节点。 |
| **记录值 / 源站** | `<ORIGIN_IP>` | 填生产源站入口的公网 IP。 |
| **回源 HOST** | 保持默认（不修改） | 默认使用客户端请求 `Host` 回源，正好匹配源站 Traefik 路由规则。 |
| **TTL** | `自动` | — |
| **高级配置（HTTP/HTTPS 端口）** | 保持默认 `80/443` | 源站由 Traefik 统一监听标准端口，无需改动。 |

点击 **下一步** 进入业务场景选择页面：

1. **业务类型**：选择 **`网站页面`**（「加速小文件和 API 请求混合的网站业务」）；不要选大文件下载/音视频点播，也不要误选带增值付费标记的选项。
2. **防盗刷功能**：暂**不一键勾选**，避免高频编辑同步或多标签并发请求触发误拦截（后续如有需要可在「安全防护」按需细化）。
3. **规则模版开关（`不使用规则模版`）**：**保持关闭（不使用模版）**。源站 [deploy/nginx.conf](../../deploy/nginx.conf) 已配好完备的 `Cache-Control` 与 `Vary` 策略，不使用预设模版可避免模板自动注入“强制全站静态缓存”或“忽略 `?v=` 查询参数”的规则。

点击 **完成** 后，记录列表中会生成该记录专属的 **CNAME 地址**（形如 `<YOUR_DOMAIN>.xx.initac.com`），复制保存备用。

### 步骤 3：修改原 DNS 解析商记录

前往域名所属的**原 DNS 解析服务商**控制台：

1. 找到主机记录对应 `<YOUR_DOMAIN>` 的原 `A` 记录；
2. 将记录类型修改为 **`CNAME`**，记录值粘贴步骤 2 生成的 ESA CNAME 地址；
3. 等待原 `A` 记录的 TTL（常见为 `600s`）过期后，全球 DNS 即自动平滑切换至 ESA 节点。

### 步骤 4：SSL/TLS 配置（防证书错误与重定向死循环）

进入站点左侧 **SSL/TLS**：

1. **边缘证书**：确认已为 `<YOUR_DOMAIN>` 开启并签发免费边缘 HTTPS 证书，等待证书状态生效。
2. **回源加密模式**：选择 **`完全（Full）`**（或更严格的端到端校验证书模式）。
   - **禁止选择「灵活 / 关闭（HTTP 回源）」**：因为源站 Traefik 强制将 HTTP 请求 `301/308` 重定向到 HTTPS；若 ESA 以 HTTP 回源，会导致用户请求陷入 `ERR_TOO_MANY_REDIRECTS` 无限重定向死循环。

### 步骤 5：缓存规则配置（含 `.wasm` 关键避坑）

> ⚠️ **实测踩坑结论**：阿里云 ESA 默认的静态后缀缓存集合包含 `.js`，但**默认不包含 `.wasm`**。如果只依赖默认配置，实测请求 `mica_rust_bg.wasm?v=...` 时边缘响应头会显示 `x-site-cache-status: DYNAMIC`——虽然仍享受了 ESA 近端接入和专线回源提速，但 `.wasm` 并未真正在边缘节点落盘缓存，每次无痕冷启动仍会回源消耗源站 ECS 出带宽。

进入站点左侧 **缓存 → 缓存规则**：

1. **新增规则 1：`Cache WASM`（核心提速 + 省回源带宽）**
   - **匹配条件**：`文件扩展名` 等于 `wasm`（或 `URI 路径` 以 `.wasm` 结尾）
   - **缓存资格 / 边缘缓存 TTL**：选择 **可缓存，遵循源站（Respect Origin）**
   - **缓存键（Cache Key）**：**必须保留全部查询字符串（Query String）**。Mica 在每次发版构建时都会生成新的内容哈希 `?v=<hex>`，保留参数才能确保发版后立即穿透更新，同时让同版本资产享受 `1 年 immutable` 边缘命中（`HIT`）。
2. **新增规则 2：`Bypass API and WS`（兜底保护）**
   - **匹配条件**：`URI 路径` 前缀匹配 `/api/` 或 `/ws/`（或使用包含匹配）
   - **缓存资格**：**绕过缓存（Bypass）**，确保认证接口、图片 `/blob` 临时重定向与 CRDT 同步永不落盘缓存。
3. **全局缓存键校验**：确认站点「缓存键」配置**未开启**「忽略所有参数（Ignore Query String）」，否则带 `?v=` 的版本区分会失效。

### 步骤 6：速度和网络协议确认

进入站点左侧 **速度和网络**，确认以下开关处于开启状态：

- **WebSocket**：必须开启（保障 `/ws/workspaces/...` 实时协同与目录通知不断连）；
- **HTTP/2** 与 **HTTP/3 (QUIC)**：推荐开启（显著降低高丢包/弱网及跨区并发资源建连延迟）；
- **Brotli 压缩**：建议开启（源站已针对 `.js` / `.wasm` 预压缩 Brotli-11 并带 `Vary: Accept-Encoding`，两者兼容）。

---

## 四、生效验收与探测命令

切流完成并等待 TTL 过期后，可按以下两套方式验收效果。

### 1. 浏览器无痕模式验证清单

打开浏览器**无痕窗口**访问 `https://<YOUR_DOMAIN>/`，按 `F12 → Network` 检查：

1. **Remote Address**：不再是源站 IP `<ORIGIN_IP>:443`，而是就近的阿里云 ESA 边缘节点 IP；
2. **Protocol**：显示 `h2` 或 `h3`；
3. **首屏 `/` 文档**：响应头带 `Server: ESA`、`Cache-Control: no-cache`、`x-site-cache-status: DYNAMIC`；
4. **`mica_rust.js?v=<hash>` 与 `mica_rust_bg.wasm?v=<hash>`**：
   - 第 1 次访问冷回源可能显示 `MISS`；
   - 第 2 次开启新无痕窗口刷新后，两者均应出现 **`x-site-cache-status: HIT`**（或 `age > 0`），且 `Content-Encoding: br`、`Cache-Control: public, max-age=31536000, immutable`；
5. **实时协同验证**：登录后确认状态徽标显示 **「已实时同步」**，Network 中的 `/ws/...` 请求状态为 `101 Switching Protocols`；同时打开包含图片的文档确认图片正常显示。

### 2. 命令行一键实测脚本

> 说明：部分 Windows 环境的自带 `curl.exe`（Schannel 后端）对新签发的边缘证书做严格离线 CRL/OCSP 吊销检查时可能报错 `CRYPT_E_REVOCATION_OFFLINE (0x80092013)`，属于客户端校验特性而非服务端故障，加 `--ssl-no-revoke` 即可完成链路探测；Linux/macOS 的 OpenSSL `curl` 与现代浏览器不受影响。

**PowerShell（Windows）：**

```powershell
$domain = "mica.example.com"   # 替换为你的域名

# 1. 验证 DNS 已切到 ESA CNAME（同时核对公共 DNS）
Resolve-DnsName $domain
Resolve-DnsName $domain -Server 223.5.5.5

# 2. 获取首页 HTML 中的静态资源哈希参数 v
$html = curl.exe --ssl-no-revoke -fsS "https://$domain/"
$ver = [regex]::Match($html, 'mica_rust_bg\.wasm\?v=([0-9a-f]+)').Groups[1].Value
echo "Asset hash: $ver"

# 3. 校验首页不强缓存 & /api/health 动态直通
curl.exe --ssl-no-revoke -sSI "https://$domain/" | Select-String -Pattern "^HTTP/|^server:|^cache-control:|^x-site-cache-status:|^via:"
curl.exe --ssl-no-revoke -sS "https://$domain/api/health"

# 4. 连续两次拉取 WASM，观察首尾耗时与 HIT 状态
curl.exe --ssl-no-revoke -sS -D - -o NUL -H "Accept-Encoding: br" -w "`n[WASM #1] ip=%{remote_ip} tcp=%{time_connect}s tls=%{time_appconnect}s ttfb=%{time_starttransfer}s total=%{time_total}s size=%{size_download}B speed=%{speed_download}B/s`n" "https://$domain/mica_rust_bg.wasm?v=$ver"
curl.exe --ssl-no-revoke -sS -D - -o NUL -H "Accept-Encoding: br" -w "`n[WASM #2] ip=%{remote_ip} tcp=%{time_connect}s tls=%{time_appconnect}s ttfb=%{time_starttransfer}s total=%{time_total}s size=%{size_download}B speed=%{speed_download}B/s`n" "https://$domain/mica_rust_bg.wasm?v=$ver"

# 5. 验证 WebSocket 升级头可透传至源站（未带 Cookie 应返回源站 401 + DYNAMIC）
curl.exe --ssl-no-revoke -sS -D - -o NUL -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" "https://$domain/ws/workspaces/00000000-0000-0000-0000-000000000000/views" | Select-String -Pattern "^HTTP/|^server:|^x-site-cache-status:|^via:"
```

**Bash（macOS / Linux）：**

```bash
DOMAIN="mica.example.com"  # 替换为你的域名

dig +short "$DOMAIN"
VER=$(curl -fsS "https://$DOMAIN/" | sed -n 's/.*mica_rust_bg\.wasm?v=\([0-9a-f]*\).*/\1/p' | head -n1)
echo "Asset hash: $VER"

curl -sSI "https://$DOMAIN/" | grep -iE '^(HTTP/|server:|cache-control:|x-site-cache-status:|via:)'
curl -fsS "https://$DOMAIN/api/health"; echo

for i in 1 2; do
  curl -sS -D - -o /dev/null -H "Accept-Encoding: br" \
    -w "\n[WASM #$i] ip=%{remote_ip} tcp=%{time_connect}s tls=%{time_appconnect}s ttfb=%{time_starttransfer}s total=%{time_total}s size=%{size_download}B speed=%{speed_download}B/s\n" \
    "https://$DOMAIN/mica_rust_bg.wasm?v=$VER"
done
```

---

## 五、实测提速基线（2026-11-28，`v0.15.5`）

以下为同一网络环境（东南亚客户端跨区访问华南国内源站）对同一份 `2,505,859 B`（`2.51 MB` Brotli-11）`mica_rust_bg.wasm?v=6390e2f368` 的切换前后实测对照：

| 评测指标 | 切流前（直连国内源站） | 切流后（ESA 专线回源 `DYNAMIC`） | 补齐 `.wasm` 缓存规则（边缘 `HIT`，`Age: 2`） | 最终提速幅度 |
| --- | ---: | ---: | ---: | ---: |
| **TCP 建连耗时** | `382 ms` | `42 ms` | **`41 ms`** | **快 9.3×** |
| **TLS 握手耗时** | `820 ms` | `58 ms` | **`57 ms`** | **快 14.4×** |
| **TTFB 首包延迟** | `1,196 ms` | `169 ms` | **`73 ms`** | **快 16.4×** |
| **下载 2.51 MB WASM 总耗时** | `3.82 s ~ 6.00 s` | `0.35 s ~ 0.65 s` | **`0.176 s`** | **快 22 ~ 34×** |
| **平均下载吞吐** | `417 ~ 656 KB/s` | `3.88 ~ 7.10 MB/s` | **`14.18 MB/s`** | **快 22 ~ 34×** |
| **首页 `/` 整页加载** | `> 1.2 s` | `0.25 s` | **`0.25 s`** | **秒开** |

> **注**：按第三节第 5 步完成 `Cache WASM` 规则配置后，第二次请求即在曼谷边缘节点命中 **`X-Site-Cache-Status: HIT`**（首次冷请求为 `MISS` 并自动回源填充），`2.51 MB` WASM 下载压至 `0.176 s`（`14.18 MB/s`），且同版本后续访问不再消耗源站 ECS 出口带宽。

---

## 六、常见问题排查与快速回滚

### 常见问题排查

| 现象 | 常见原因 | 处理办法 |
| --- | --- | --- |
| 控制台站点已「已启用」，但流量/请求数始终为 `0` | 原 DNS 解析商仍保留 `A` 记录，或本机/运营商 DNS TTL 尚未过期 | `Resolve-DnsName` / `dig` 查权威或 `223.5.5.5`，确认返回 `CNAME`；本机可等原 TTL 过期或刷新 DNS 缓存 |
| 刚切换 CNAME 后偶发 HTTPS TLS 握手报错 | 边缘节点刚刚切通，边缘免费证书尚未完成全网分发，或 Windows Schannel 报 `CRYPT_E_REVOCATION_OFFLINE` | 确认 ESA「SSL/TLS → 边缘证书」已生效；若仅 `curl.exe` 报错而浏览器正常，探测时加 `--ssl-no-revoke` |
| 浏览器提示 `ERR_TOO_MANY_REDIRECTS` | ESA 回源加密模式错选为「灵活（HTTP 回源）」，与源站 Traefik 强制 HTTPS 冲突 | 将 ESA 回源模式改为 **完全（Full）** |
| `.wasm` 返回 `x-site-cache-status: DYNAMIC` 不命中缓存 | ESA 默认静态扩展名不包含 `.wasm` | 按 [步骤 5](#步骤-5缓存规则配置含-wasm-关键避坑) 添加扩展名 `wasm` 缓存规则（遵循源站 + 保留 Query String） |
| 发版后浏览器仍加载到旧版 UI 或 WASM | 误将 `/`（`index.html`）强制缓存，或缓存键设置了「忽略查询字符串」 | 检查确认 `/index.html` 为 `no-cache`（`DYNAMIC`），且静态资源缓存键保留全部 `?v=<hash>` |
| 登录后页面一直无法进入「已实时同步」或协作延迟/断连 | ESA 未开启 WebSocket，或安全防护规则拦截高频 WS/同步请求 | 检查 **速度和网络 → WebSocket** 是否开启；检查「安全防护」拦截日志并适当放宽同步接口规则 |

### 快速回滚（秒级切回源站）

若需紧急绕过 CDN，只需在**原 DNS 解析商**处将 `<YOUR_DOMAIN>` 的 `CNAME` 记录改回指向 `<ORIGIN_IP>` 的 `A` 记录即可（无需改动源站代码与容器配置）；建议长期稳定运行前将原 DNS 与 ESA 侧记录 TTL 设在 `300~600s` 以兼顾稳定性与切换速度。

---

## 七、对象存储 RustFS 域名（`s3.<YOUR_DOMAIN>`）接入 ESA 差异与避坑

> 例如生产环境中的 `s3.mica.cloudcele.com`。对象存储承接的是 **AWS SigV4 预签名直传（`PUT`）** 与 **带签名查询参数的外链读取（`GET`）**，接入流程与主站域名相似，但**规则配置不能直接照搬主站**。

### 1. 同源配置（与主站一致的部分）

- **复用已有 ESA 站点零成本接入（推荐）**：当 `s3.mica.<DOMAIN>` 与主站 `mica.<DOMAIN>` 同属一个父域时，若主站本身为 **ESA 基础版或以上站点**，**无需为 `s3.` 再单独购买一套付费套餐**——直接在已有父域 ESA 站点下新增一条 A/AAAA 回源记录（主机记录填 `s3`，开启代理，目标为 `<ORIGIN_IP>`），通过主/子域名 Cache Rule 精确隔离缓存行为即可；
- **独立站点方案**：若主站在免费版下不承载通配子域或需要独立隔离，可在 ESA 新增站点 `s3.<YOUR_DOMAIN>`（CNAME 模式、全球加速），DNS 回源填 `<ORIGIN_IP>`、`@${host}`（即 `@`），代理加速开启；
- **SSL/TLS**：开启边缘证书（同一基础版站点 DigiCert 单域名配额已被主站占用时，可为 `s3.` 申请免费的 **Let's Encrypt** 边缘证书），回源加密模式选 **完全（Full）**（源站 Traefik 路由 `mica-s3` 对该域名提供 HTTPS 终止）；
- **缓存隔离规则**：若复用主站站点，必须添加专用 Bypass 规则（如 `(http.host eq "s3.mica.<DOMAIN>")` → **全部绕过缓存** + 缓存键 **保留全部查询字符串**），确保预签名 `PUT`/`GET`/`OPTIONS` 永不与主站静态缓存规则混淆。

> **生产实况（2026-11-28）**：`s3.mica.cloudcele.com` 已按「复用基础版站点 `191392438304884`（`mica.cloudcele.com`）」零增量成本方案接入 ESA（代理 CNAME `s3.mica.cloudcele.com.a2.initac.com` + `Bypass S3 Presigned` 规则 + Let's Encrypt 证书）。

### 2. 必须严格区分的 5 大核心差异（红线清单）

| 维度 | 主站（`mica.<DOMAIN>`） | RustFS 对象存储（`s3.mica.<DOMAIN>`） | 错误后果 |
| --- | --- | --- | --- |
| **业务类型 / 规则模版** | 网站页面（关闭模版） | **网站页面**或仅创建裸记录（**务必关闭规则模版 & 关闭一键防盗刷**） | 误开大文件模版或严格 WAF 规则可能拦截 `PUT` 二进制图片流或重写缓存键 |
| **允许请求方法与上传体积** | 主要是 `GET`/`POST`/WS | **Browser 直传必须放行 `PUT` / `OPTIONS` / `GET` / `HEAD`**，注意单文件最大上传放宽（默认 `20 MB` 内网图无压力，WAF 不要拦截大包 binary `PUT`） | 编辑器粘贴/上传图片时在预签名 `PUT` 阶段直接报错 `403` / `405` |
| **回源 `HOST` 必须原样透传** | 匹配 Traefik | **严禁改写 `Host` 头**，必须保留 `s3.mica.<DOMAIN>` | SigV4 将 `host` 签入 `X-Amz-SignedHeaders`，改动即触发 RustFS `403 SignatureDoesNotMatch` |
| **缓存键 Query String** | 保留 `?v=` 哈希参数 | **严禁勾选「忽略全部查询字符串（Ignore Query String）」** | 预签名 URL 靠 `?X-Amz-Algorithm=...&X-Amz-Expires=...&X-Amz-Signature=...` 鉴权；忽略参数会让无签名/过期请求误命中缓存造成越权 |
| **CORS 响应头** | 同源为主 | **不要在边缘剥离或覆写源站返回的 `Access-Control-*` 头**，且对 `OPTIONS` 预检请求不应缓存错误结果 | 主站前端直传 `PUT` 到 `s3.` 子域名跨域失败 |

### 3. 对缓存命中率的合理预期

- `/api/workspaces/{ws}/files/resolve` 每次签发的 RustFS `GET` URL 都带有不同时间戳和 `X-Amz-Signature`。只要严格保留完整 Query String，**不同签名的请求之间默认不会跨会话复用缓存**；
- 只有单标签页/短期内重复请求同一签发 URL（或公开外链稳定 URL）能命中边缘 `HIT`；
- 因此 `s3.` 接入 ESA 的主要收益是 **近端边缘 TLS 握手加速 + 跨境专线稳定回源（解决跨国传图/看图慢和丢包断流）**，而不是静态高命中率。若确实只代理单次 `GET` 而不经过签名校验才考虑签名剥离（私有工作区文档图片严禁剥离）。

---

## 八、主域名切入 ESA 后 Ansible / CI SSH 部署避坑

- [ansible/inventory.yml](../../ansible/inventory.yml) 中生产主机若填写了 `ansible_host: mica.cloudcele.com`，一旦主域名 CNAME 指向阿里云 ESA 代理 IP，**ESA 默认只代理 80/443（HTTP/HTTPS）端口，不转发 SSH 22 端口**，会导致 GitHub Actions `Deploy` 任务在 SSH 连接阶段超时失败。
- **标准修法（任选其一，推荐方案 A）**：
  1. **方案 A（推荐，保留域名语义与独立 SSH 入口）**：在原 DNS 解析商新增一条**不经 ESA 代理的直连 `A` 记录**（如 `ssh.mica.cloudcele.com` 或 `origin.mica.cloudcele.com` → `<ORIGIN_IP>`），然后将 [ansible/inventory.yml](../../ansible/inventory.yml) 的 `ansible_host` 改为该直连域名，并按新主机名同步更新 GitHub 仓库 Secret `DEPLOY_KNOWN_HOSTS`（`ssh-keyscan -H <新主机名>`）。
  2. **方案 B（最简）**：直接将 `ansible_host` 改为 `<ORIGIN_IP>`，并用源站 IP 的主机公钥条目更新 GitHub Secret `DEPLOY_KNOWN_HOSTS`。
