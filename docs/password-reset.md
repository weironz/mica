# 密码找回（忘记密码）

公开发布合规 B#3。陌生用户忘了密码的自助出路:请求 → 收邮件 → 点链接 → 在网页上设新密码。

## 流程

1. 登录页点「忘记密码?」→ 填邮箱 → `POST /api/auth/password/forgot {email}`。
   - **永远返回 204**,不论邮箱是否注册(不做账号枚举 oracle)。注册了才发信。
2. 服务端 mint 一个单次性 token(前缀 `mica_pr_`,存 sha256,1 小时过期,同一用户
   再次请求会删掉旧 token —— 只有最新链接有效),发一封含
   `{MICA_APP_BASE_URL}/reset-password?token=…` 的邮件。
3. 用户点链接 → **服务端渲染的无 JS 网页** `GET /reset-password`(和 `/s/` 分享页同款:
   挂在 `/api` 之外、nginx 代理、严格 CSP)→ 填新密码 → `POST /reset-password`。
4. POST 用一条**条件 UPDATE** 原子地花掉 token(用过/过期的链接无法重放),写新
   `password_hash`,并 `revoke_user_sessions`(重置密码 = 怀疑账号被人拿了,所有会话一起杀,
   与 change_password 同规)。成功页提示去 App 重新登录。

客户端(桌面/web)**只做「请求发信」这一步**;重置本身永远在浏览器网页里完成 —— 邮件链接
在用户任意浏览器打开,App 不接深链。

代码:`crates/api-server/src/routes/password_reset.rs`(端点 + 页面 + token),
迁移 `migrations/0013_password_reset_tokens.sql`,发信抽象 `crates/infra/src/mail.rs`
(trait + LogMailer),DirectMail 实现 `crates/api-server/src/mail.rs`。

## 发信后端

默认 **LogMailer**:不真发信,把整封邮件(含重置链接)打到 server 日志。**这样整条流程
在没配任何邮件服务商时就能跑通** —— 运维从日志里捞链接即可,dev/test 也不碰网络。

生产走 **阿里云 DirectMail(邮件推送)**:节点在阿里云,阿里云默认封 25 端口出站,DirectMail
是第一方、发信量在免费额度内(200 封/天,密码重置远用不满)。走它的 HTTP API(`SingleSendMail`,
v1 RPC 签名 HMAC-SHA1),复用 `reqwest`,只加了 `sha1`/`hmac` 两个小 crate,不引 `lettre`。

### 一次性配置(在阿里云控制台,只能你来做)

1. 开通**邮件推送 DirectMail**。
2. **验证发信域名**:控制台加一个发信域名(如 `mail.cloudcele.com`,`cloudcele.com` 你已拥有,
   不用另买),按提示在 DNS 加 SPF/DKIM/(可选 MX、_dmarc)记录,等验证通过。
3. 建一个**发信地址**(如 `noreply@mail.cloudcele.com`)。
4. 建一个 **RAM 用户 + AccessKey**,授权 DirectMail(`AliyunDirectMailFullAccess` 或更细的发信权限),
   记下 AccessKeyId / AccessKeySecret。

### 节点 `.env`(`/data/mica/.env.prod`)填这几项后 `deploy-prod` 即生效

```env
MICA_MAIL_BACKEND=directmail
MICA_MAIL_FROM=noreply@mail.cloudcele.com     # 上面建的发信地址(DirectMail AccountName)
MICA_MAIL_FROM_NAME=Mica                       # 可选,收件人看到的发件人显示名
MICA_MAIL_ACCESS_KEY_ID=LTAI...
MICA_MAIL_ACCESS_KEY_SECRET=...
MICA_MAIL_REGION=cn-hangzhou                    # 默认即此,一般不用改
# MICA_MAIL_ENDPOINT=https://dm.aliyuncs.com/  # 默认即此,除非用区域化 endpoint
MICA_APP_BASE_URL=https://mica.cloudcele.com   # 重置链接的域名前缀(compose 默认 https://<DOMAIN>)
```

配置**缺项不会崩**:`MICA_MAIL_BACKEND=directmail` 但必填项缺失时,启动 WARN 并回退
LogMailer(重置链接仍能从日志里拿到)。

env 清单同时在 `deploy/docker-compose.yml` 的 api service 里有注释。

## 限流 & 安全

- `/auth/password/forgot` 和 `/reset-password` 都进了 per-IP token bucket(`rate_limit.rs`)。
  两者都**不**占 Argon2 并发闸:forgot 不 hash;reset 只有在 token 有效花掉之后才 hash,
  没 token 的攻击者根本到不了 hash。
- token 只存 sha256、单次性、1 小时过期;重置成功杀掉该用户所有会话。
- 重置页无 JS、严格 CSP(`default-src 'none'`),token 反射进表单前做 HTML 转义。
- forgot 恒定 204 + 发信失败只记日志不改响应 → 不泄露邮箱是否注册。

## 测试

- `password_reset::reset_pg`(DATABASE_URL 门控):单次性、"新请求作废旧链接"、过期拒绝。
- `mail.rs`:DirectMail 签名的确定性 + percent-encoding 规则;`password_reset::tests`:转义 + 邮件含链接。
- 端到端真发信只能配好 key 后验证(签名对不对最终以 DirectMail 是否接受为准)。
