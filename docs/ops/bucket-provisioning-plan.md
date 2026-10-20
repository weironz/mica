# 建桶该由谁做

2026-08-06。起因:`deploy/docker-compose*.yml` 里的 `rustfs-init` 把 RustFS 的实现细节焊死了 ——
它假设「bucket = `/data` 下的目录」、假设镜像里有 `rustfs` 这个用户、还多拉一个镜像。换 MinIO
勉强能改,换 AWS S3 或阿里云 OSS 则完全无从谈起:那两个根本没有容器可以 `exec`。

用户提的方向:把建桶挪进代码,只依赖 S3 接口。本文是调研 + 方案。

## 1. 同类产品怎么做

| 项目 | 做法 | 结果 |
| --- | --- | --- |
| **Gitea** | `BucketExists` → 不存在就 `MakeBucket` → **失败即拒绝启动**,无开关 | 桶不存在时能自愈 |
| **Nextcloud** | `doesBucketExist` → `createBucket`,由 `verify_bucket_exists` 控制(默认 true) | **翻车**,见下 |
| **AFFiNE** | 存储客户端里没有任何建桶代码;自托管默认走 `Fs` 本地存储,压根不碰 S3 | 问题不存在 |
| **Synapse / GoToSocial** | 不建桶 | 受限凭据下一切正常 |

关键证据是 Nextcloud [issue #36427](https://github.com/nextcloud/server/issues/36427):用户用 StorJ、
凭据只限单个桶、`autocreate` 已设为 false,Nextcloud 仍然去建桶 → `Access denied`。报告人原话:

> 其他 S3 兼容软件如 GoToSocial、Matrix Synapse 完全正常

**"不建桶"的实现在受限凭据下反而更兼容。** 这正是 CLAUDE.md 第 6 条要找的"它刻意没用什么" ——
所以本方案抄的不是 Gitea 的动作,而是它们共同的**失败姿态**。

## 2. 技术前提(验过,不是推测)

调 S3 接口建桶可行,但有三处必须显式处理,否则换云就炸:

1. **AWS 桶名是全局唯一命名空间**。默认桶名 `mica` 在 AWS 上必然已被别人占用 →
   `BucketAlreadyExists` 409。**自动建桶在 AWS 上基本没用**,只对自托管对象存储有意义。
2. **`BucketAlreadyOwnedByYou` 是 409,但 us-east-1 为了向后兼容返回 200**。两种都要当成功。
3. **阿里云 OSS 的 S3 兼容层支持 `PutBucket`**(同账号同地域上限 100 个桶),所以 OSS 可行。

另外 `s3:CreateBucket` 是一项**额外权限**。Mica 的架构是浏览器直接 presign,服务端本来只需要
对象级权限 —— 为省一次性的建桶而让每个部署的凭据永久多一项权限,方向是反的。这条不改变结论
(自托管场景凭据本来就是全权的),但它决定了**建不了桶时绝不能崩**。

## 3. 我们自己的约束

- `crates/infra/src/storage.rs` 是**纯 SigV4 签名器**:没有 HTTP 客户端、没有 async,只签 URL
  交给浏览器。
- 但 `crates/api-server/src/blob_gc.rs` 已有先例:用预签名 URL + `reqwest` 在服务端发请求删孤儿 blob。
- 签名用的是 `UNSIGNED-PAYLOAD` 且只签 `host`,**所以桶级 PUT 带 `LocationConstraint` 的 XML body
  不影响签名**。

结论:复用现成机制即可,**不需要引入 AWS SDK**,也不需要给 `mica-infra` 加 `reqwest` 依赖。

## 4. 方案

启动时执行,位置与 `ensure_jwt_secret` 对称(`main.rs`,配置之后):

```
HeadBucket
├─ 200                      → 完成
├─ 404 / NoSuchBucket       → 尝试 CreateBucket
│                               ├─ 2xx                     → 完成(已创建)
│                               ├─ BucketAlreadyOwnedByYou → 完成(并发启动时的正常竞态)
│                               ├─ AccessDenied            → ERROR:桶不存在且这套凭据建不了
│                               └─ BucketAlreadyExists     → ERROR:桶名已被他人占用(AWS 全局命名空间)
└─ 其他一切(403 / 超时 / 连不上) → 当作"存在但我无权探测",继续启动
```

**最后一条是整个方案的要害**,也正是 Nextcloud 栽的地方:探测失败绝不能升级成建桶或崩溃。

### 两个刻意的取舍

- **不阻断启动**(Gitea 会)。Mica 现在 `S3Config::from_env()` 返回 `None` 就关掉文件功能、
  照常启动 —— 保持一致。把可恢复的配置错误变成整站宕机不划算。
- **不加开关**。Nextcloud 需要 `verify_bucket_exists` 是因为它探测失败会炸;上面"任何非 404
  都继续"已经覆盖受限凭据场景,再加旋钮就是为假想需求预留(CLAUDE.md「不要过度设计」)。

### 刻意不做

- **不通过 API 设 ACL / policy / CORS**。RustFS 的 CORS 走环境变量,AWS 要 `PutBucketCors`,
  两边形态不同且都要额外权限。当前部署已由 compose 处理,不进这一层。
- **不做运行时重试**。启动探测一次即可;真出问题时上传会自己报错,而日志已经说过了。

## 5. 一句实话:今天这不修任何 bug

不换存储的话,`rustfs-init` 在当前 compose 下工作正常,而且当前 compose 总是把 rustfs 一起拉起来,
本来也不支持指向外部 S3。这个改动的价值是两条:

1. 去掉一处焊死 RustFS 实现细节的 hack;
2. 以后真要换 MinIO / OSS / S3 时,这块不用重做。

代价是:少一个容器,多一次启动期网络调用,以及**第一次**在服务端写桶级 S3 代码。

## 6. 测试

决策树的每个分支都要有测试,**光测 happy path 等于没测**。做法是把**策略**与 **IO** 分开:

- 纯函数 `classify_head(status)` 与 `classify_create(status, body)` 可直接单测,
  覆盖 200 / 404 / 403 / 409(两种 409 分别断言)。
- 签名器新增的桶级 URL 有固定输入的签名断言(与既有 `presign` 测试同形)。
- 端到端:全新卷 + 空 rustfs,起栈后断言桶被创建、上传 200。

## 7. 工作量

**M**(约半天)。大头不在签名,在错误分类和测试。

## 8. 实施中挖出的:服务端需要自己的 endpoint(计划里没有)

第一次实测桶没被建出来,日志是"探测发不出去"。根因不在建桶逻辑,而在一个**本方案之前就存在的
隐含假设**:

`S3_ENDPOINT` 按设计是**浏览器可达地址**(compose 原注释:"Must be BROWSER-reachable —
presigned URLs embed this host")。**容器内部能不能到那个地址是另一个问题** ——
单机栈里它可能是公网 IP(要靠 NAT 发夹回来),Traefik 栈里它是 `https://<S3_DOMAIN>`
(要 DNS + TLS + 绕回 Traefik),而 rustfs 明明就在同一张网络上一跳之外。

**`blob_gc.rs` 早就在用同一个地址删孤儿 blob** —— 也就是说这个脆弱性一直都在,只是它的失败
没有症状:浏览器上传照常成功,只有服务端那两条路悄悄够不着。

修法:`S3_INTERNAL_ENDPOINT`(可选,未设回落到 `S3_ENDPOINT`,故对现有部署兼容),两份 compose
都设成 `http://rustfs:9000` —— 这本来就是 backup sidecar 已在用的地址。客户端预签名
(`presign_put`/`download_url`)继续用公网地址,服务端的(桶级操作 + `presign_delete`)走内部地址。
有测试钉住这个方向,**因为搞反了同样没有症状**。

教训与 §1 同源:**"能跑"不等于"没把某个前提焊死"**。这里被焊死的前提是"浏览器能到的地址服务端
也能到"。
