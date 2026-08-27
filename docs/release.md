# 发版与构建(Release & build)

**一句话**:**CI 构建一切,上线仍然要人按一下 —— 但那个按钮现在在 Actions 页面。**

推一个 `v*` tag,GitHub Actions 产出全部 7 个产物;之后手动触发 `Deploy` workflow,
生产滚到那个版本。两步,没有第三步。

## 你要按的那几下

整个流程里手动的部分只有这些。照抄即可,`X.Y.Z` 换成版本号(补丁位递增;minor 由用户拍板)。

```bash
# 1. bump → 门禁 → commit → tag。一条命令，门禁夹在中间，跳不过去。它不推送。
#    要本地 Postgres 起着（just dev），否则当场拒绝——理由见步骤 3。
just release X.Y.Z

# 2. 推送。发布列车从这一下开始，CI 产出全部 7 个产物（约 10–15 分钟）。
git push origin main vX.Y.Z

# 3. 等 CI 绿，并确认 4 个 asset 都挂上了（草稿 release 在最后一步才翻成正式版）
gh run watch --repo weironz/mica
gh release view vX.Y.Z --repo weironz/mica

# 4. 彩排。对真节点跑一遍整个 playbook，不改任何东西，逐行 diff 出要做的改动。
#    docker 那几步是 community.docker 原生模块，所以彩排跑的是同一段代码，不是跳过。
gh workflow run Deploy --repo weironz/mica -f version=X.Y.Z -f check=true

# 5. 上线。没有审批门——触发本身就是那个决定。
gh workflow run Deploy --repo weironz/mica -f version=X.Y.Z
```

`verify-prod`(断言 `/api/health` 报的就是这个版本)**已内置在 workflow 里**,不用单独跑。

### 机器替你把关的(以前靠记性,现在会拒绝)

| 关的什么 | 在哪 | 何时加的 |
|---|---|---|
| 三处版本号一致 + `Cargo.lock` 跟上 | `scripts/release-check.sh` | 2026-08-25 |
| **本机 Flutter == `.fvmrc`;本机 Rust == `rust-toolchain.toml`;三个镜像同号** | 同上 | 2026-08-27 |
| DB 集成测试**真的跑了**(本地 Postgres 没起就拒绝发版) | 同上 | 2026-08-25 |
| `cargo test` / `clippy -D warnings` / `flutter analyze` + `test` | 同上 | 2026-08-25 |
| **tag 必须与它所在那棵树的版本号一致** | `.github/workflows/release.yml` | 2026-08-26 |
| **带迁移的发布,上线前自动落 `pg_dump` 还原点** | `ansible/deploy.yml` | 2026-08-26 |
| 部署失败自动复原 `MICA_VERSION` | 同上 | 2026-08-25 |

### 只有三件事机器做不了

1. **发不发、发哪一位** —— 用户决策。
2. **同步 `docs/roadmap.md`**(步骤 5)。`just release` 结尾会提醒,但**故意不做成门禁**:
   不是每版都关条目,老响的警报会被无视。漏了的代价是文档变旧,不伤数据。
3. **冒烟测这一版真正改了什么**(步骤 11)。`verify-prod` 只断言版本号 ——
   **版本号证明不了功能**。

> **上面这 5 步不需要本地装任何东西** —— 全部是 `git` 和 `gh`,ansible 跑在 CI 的 runner 上。
>
> **兜底 `just deploy-prod` 只在 Linux 上跑**:ansible 不支持 Windows 作控制端
> (`pipx install ansible-core` 装得上,一运行就 `OSError: [WinError 87]`)。Linux / macOS /
> 任意 WSL 发行版都可以;Docker Desktop 自带的 `docker-desktop` 不是发行版,不算。
> `scripts/deploy-prod.sh` 检查的是 ansible **能不能运行**,不是文件在不在 —— 后者在 Windows
> 上会通过,然后崩在 ansible 自己的启动里。

`just deploy-prod X.Y.Z` 和 CI 走**同一条**路径 —— 两边都是 `ansible/deploy.yml`,CI 那一步
字面上就是在跑 `bash scripts/deploy-prod.sh`。「compose 改了必须走兜底」这条**已经不成立**
(2026-08-25,见下节)。

| 产物 | 谁构建 | 去哪 |
|---|---|---|
| `Mica-Setup-X.Y.Z.exe` | **CI** job `windows` | GitHub Release(驱动应用内自动更新) |
| `mica-cli` ×3(win/linux/macos) | **CI** job `cli` | GitHub Release |
| `mica-api` / `mica-web` / `mica-cli` 镜像 | **CI** job `images` | **阿里云 ACR**(生产拉这里)+ Docker Hub(异地副本) |
| 生产上线 | **你触发** `Deploy` workflow(兜底:`just deploy-prod X.Y.Z`) | node72 从 ACR pull |

## CI 拿的是 root key —— 这是一次有代价的取舍

**2026-08-25 之前**这一节讲的是相反的事:CI 那把密钥被 `authorized_keys` 的
`restrict,command=` 钉死在 `/usr/local/sbin/mica-deploy` 上,泄漏了也只能把生产在
**已发布版本之间**挪 —— 开不了 shell、读不了 `.env`(JWT_SECRET / 库密码 / OSS AK)、
`pg_dump` 不了、改不了 compose。那个设计是对的,论证也是对的(「能部署 ⇒ 需要 root
key」确实是个没验证过的前提,正是 CLAUDE.md 原则 #6 说的那类「必须 X 才能 Y」)。

**现在拆掉了。** 完整的账在 [`cd-plan.md` §4.1](cd-plan.md),这里只留结论:

- 栅栏**关的是一间两扇门房间里的一扇** —— 同一个 CI 手里握着 `ACR_PASSWORD`,被攻破
  的 CI 可以推一个恶意 `mica-api:v0.13.28`,再用那把「只能部署已发布版本」的密钥
  正大光明地部署它,栅栏全程不反对。
- 栅栏的**代价是一条真的坏掉的发布路径**:节点自存一份 compose、CI 传 tag 上那份的
  sha256、不一致就拒绝 —— 于是**每一次 compose 改动**(配额、开关、备份变量)都让下
  一次部署失败,直到有人上笔记本手工兜底。于是兜底路径变成了实际主路径:一条没人设计
  过、没人测过、CI 跑不了的主路径。
- Ansible 把模块推到主机上执行,**天然不能钉死在一条命令后面**。要 Ansible 就得放弃
  栅栏,没有中间态。

**净损失,别粉饰**:`DEPLOY_SSH_KEY` 现在是一把有 shell 的 root key,被攻破的 CI 能在
节点上做任何事,而不只是换版本号。想找回一部分:给 Ansible 用非 root 用户 + 限定范围的
sudo 规则,或者改成节点侧拉取。两个都没做 —— 都要等 provisioning 那一层先存在
(`cd-plan.md` §3)。

**上线仍然是你按的一下**:`deploy.yml` 只有 `workflow_dispatch`,没有 `on: push` ——
触发本身就是那个决定,只是按钮从你的终端挪到了 Actions 页面。

`environment: production` 保留着,但**故意不配 required reviewer**:触发已经是人工动作,
再加一道审批就是对同一个决定确认两次,对单人项目是摩擦不是安全。哪天不再是单人,
在 Settings → Environments 里加审批人即可,workflow 不用改。

### compose 不一致这个问题是怎么消失的

不是「加了个更好的比对」,是**把可比对的东西删掉了**。

节点上的 `docker-compose.yaml` 现在**每次部署都从 tag 发过去**
(`git show v<version>:deploy/docker-compose.yml`)。节点不再保管一份「自己的」compose,
所以「漂移」这个概念没有载体 —— 它按构造就是这一版发布时的那一份,没有东西可比。

> ⚠️ **反面同样成立,而且更容易伤到人:在节点上手改 `/data/mica/docker-compose.yaml`
> 会被下一次部署静默抹掉。** 改动要进仓库、发一版。旧的 backup 文件会留下
> (`ansible.builtin.copy` 带 `backup: true`),但没有任何东西会提醒你它被覆盖过。
> 具体会踩到的场景见 `docs/deploy.md` 的 PostgreSQL 大版本升级一节 —— 那套步骤原本
> 就是让你直接编辑节点上那个文件的。

⚠️ **compose 一定要从 tag 取,不是工作树。** `scp deploy/docker-compose.yml` 曾经用一个
已经往前走的分支去部署 0.12.6,给生产发了**另一个版本**的 compose,而且静默成功。所以
`ansible/deploy.yml` 的 `compose_src` **故意没有默认值**:裸跑 playbook 会带着说明拒绝,
必须由 `scripts/deploy-prod.sh` 用 `git show` 取出来传进去。

### 节点上的环境变量:`.env` 渲染,`.env.secrets` 人工

2026-08-25 起,`/data/mica/.env` 拆成两个文件:

| 文件 | 权限 | 内容 | 谁维护 |
|---|---|---|---|
| `.env` | 640 | 21 个非凭据配置 + `MICA_VERSION` | **ansible 每次部署从 `ansible/host_vars/mica-prod.yml` 渲染** |
| `.env.secrets` | 600 | 13 个凭据 | **人工,装机时一次**;ansible 从不读、不写、不复制 |

做这件事的理由**不是藏密钥**,是那 21 个配置值(配额、注册开关、备份时间、邮件后端、
CORS、token TTL……)**此前只存在于节点上**,仓库里没有真相源,也无从判断有没有被手改过 ——
和 compose 当初一模一样的病。拆开之后配置那半才敢覆盖:凭据那半不会被碰。

改配置**改 `host_vars`,不要改节点** —— 节点上的改动活到下次部署为止。

两道护栏,都是拒绝而不是提醒:

- **`host_vars` 里出现凭据形状的键(`secret|password|token|access_key`)直接拒绝部署**
  —— 那是公开仓库里的文件。
- **节点上现有的键,一个都不许在渲染后消失** —— 少一个键不会报错,compose 会**静默**用
  自己的默认值顶上,配额或开关就这么被悄悄改掉了。

**验证方式(以后改这块照做)**:比较 `docker compose config` 解析出的完整配置的 sha256。
拆分前后必须**逐字节相同** —— 这能在不看任何密钥值的前提下证明改动是 no-op。本次:
`9a820b14…` 前后一致。

> ⚠️ **在节点上一律用 `./dc`,不要裸敲 `docker compose`。**
> `docker compose` 只自动读 `.env`,而 Mica 的 compose 把凭据写成 `${VAR}` 而不是
> `${VAR:?}` —— 所以**它不会报错**,凭据解析成空字符串,命令照常成功:api 会带着空的
> `JWT_SECRET` 起来(服务端自铸一把新的 → **所有会话被登出**)和空的库密码(→ 连不上库)。
> 改成 `${VAR:?}` 能让它响,但那些默认值是给自托管者「零凭据起栈」用的、是刻意的
> (`docs/deploy.md`),为了保护一台机器上的运维便利去改它,是把成本转嫁给所有人。
> 所以护栏就是 `./dc` 加这段话 —— 这是一处**真实存在的锋利边缘**。

### playbook 保证的事(`ansible/deploy.yml`)

按顺序,每一条都是踩出来的:

1. **动任何东西之前先拒绝** —— 版本号格式、compose 存在、节点 `.env` 存在、`.env` 里有
   `MICA_VERSION` 可供回滚。全部前置,坏输入不会留下半应用状态。
2. **`.env` 写 `MICA_VERSION=v<version>`,带 `v`** —— compose 直接拿它当镜像 tag
   (`mica-api:${MICA_VERSION}`),少写 `v` 就是去拉一个不存在的 tag。彩排抓到的。
3. **只 pull app 服务(api/web)** —— 不是优化:节点**连不上 `registry-1.docker.io`**,
   pull 全部会 `i/o timeout` 直接失败。postgres/rustfs 是第三方固定镜像,早在节点上,
   也不随版本走。
4. **数据面只查不动** —— `docker compose up -d postgres` 会因为 config-hash 不一致
   **重建 `mica-postgres-1`**,等于每次发版弹一次库(旧脚本写 `--no-deps api web` 正是
   为了这个)。改成:确认它在跑且健康;**不健康就拒绝部署**(带病部署会把基础设施故障
   伪装成一次坏发版);只有它压根没起来才启动它(新机器 / 重启没拉回来)。
5. **app 服务 `--no-deps` + 等健康** —— `wait` 换掉了原来手写的 `for i in $(seq 1 60)`
   轮询,那个 go-template 转义错了,对着健康的部署喊 "api NOT healthy"。
6. **backup sidecar 只刷新已存在的** —— 它跑 mica-cli、吃同一个 `MICA_VERSION`,不跟着
   滚就漂(曾停在 `willdockerhub/mica-cli:v0.3` 好多个版本);但和别的一起 `up` 会把
   backup 从关变开,所以按 profile 单独处理。
7. **任何一步失败就回滚 `MICA_VERSION`** —— 否则 `.env` 停在一个容器根本没起来的版本上,
   下一次重启(或 OOM)就按它回来。compose **不**跟着回滚:新版本镜像配旧 compose
   (或反过来)是半个回滚,比不回滚更坏。

## 镜像与 tag:两条硬规矩

1. **三个镜像必须同版本一起推**。compose 里 api / web / **cli**(`backup` 服务)全部
   吃同一个 `${MICA_VERSION}`。少推一个,那个服务就拉不到镜像。CI 的矩阵保证了这点。
2. **tag 永远不可变(`v0.5.1`),绝不用滚动 tag / `latest`**。
   历史教训:生产曾靠第三方拉取加速器 `docker.1ms.run` 访问 Docker Hub,而这类 mirror
   **会缓存 tag→digest** —— 滚动 tag(过去是 `v0.3` 一路滚到 v0.5)存在"推了新镜像、
   拉下来还是旧的"的真实风险。换成 ACR 一方仓库 + 不可变 tag 之后,这类问题从根上没了:
   一个从未存在过的 tag,无从被缓存成旧值。

> 顺带更正一个流传过的错误说法:**节点并非连不上 Docker Hub**(实测 `docker pull` 通,
> 走的是 daemon 里配的 `docker.1ms.run` 加速器)。以前那套 `docker save | scp | load`
> 经笔记本中转 90MB 的搬运,是建立在这个过时假设上的,现已删除。

## 完整发版流程

> **第 1–3 步和第 6 步的一半,现在是一条命令:**
>
> ```bash
> just release X.Y.Z
> ```
>
> bump → 门禁 → commit → tag,一步做完,**门禁夹在中间**。这个顺序不是可选的:
> `release-check` 断言三处版本号一致,而它们只有 bump 之后才一致 —— 老的 doc string
> 写着「先跑 release-check」,那个顺序根本过不了。
>
> 它**不推送**。推送是对外不可逆的一步,留成显式的 `git push origin main vX.Y.Z`。
>
> 下面第 1–3 步保留是为了说清它在做什么、以及为什么必须这么做,不是让你手动照做。

1. **版本号三处同步**(必须一致):
   - `clients/mica_flutter/pubspec.yaml` 的 `version:`
   - `clients/mica_flutter/lib/main.dart` 的 `kAppVersion`
   - 根 `Cargo.toml` `[workspace.package]` 的 `version`(**api-server 与 mica-cli 都 `version.workspace=true` 继承它**——改这一处两个二进制的 `/api/health` / `--version` 一起对;顺带 `cargo check` 更新 `Cargo.lock`)
2. **判断服务端要不要跟着发**:改动是否触及 `crates/markdown` 等服务端依赖?
   链路 `api-server → mica-app-core → mica-markdown`。用 `cargo tree -p <crate> | grep <dep>`
   实证,别猜。(例:v0.5.0 的 CJK 强调改了 markdown → api 必须重建。)
3. 测试全绿,**且是真的跑过**。DB 集成测试在 `DATABASE_URL` 没设时**静默跳过**,而
   跳过的测试报告为「通过」—— 整套 0.00s「全过」,那是真空通过不是验证
   (见 `docs/lessons.md`)。

   这条规则以前靠人记,记不住,于是被写进 CLAUDE.md 当提醒 —— 而**一条需要提醒的规则
   不是规则,是指望**。现在它是 `scripts/release-check.sh` 的第一条:本地 Postgres
   没起来就**当场拒绝发版**,不是警告后继续。`just release` 必经这一步,跳不过去。

   (单独跑:`just release-check`。它按 CI 的方式跑同样的门 —— `cargo test --workspace`
   带 `DATABASE_URL`、`clippy -D warnings`、`flutter analyze` + `flutter test`,
   外加 `Cargo.lock` 不能是脏的。)
4. **桌面端带本地库迁移时**,先跑一次真库升级冒烟:
   `MICA_REAL_STORE=<一份真 store.db 的拷贝> cargo test -p mica-core --features store -- --ignored upgrade_real_store_smoke`。
   它默认 `#[ignore]`、要手动设环境变量,所以不写进这里就等于不存在 —— 而桌面
   自动更新后**首启就地迁移本地库**,迁移写坏 = 用户笔记打不开。改过
   `crates/mica-core` 的 store schema / `SCHEMA_VERSION` 时必跑。
5. **同步 `docs/roadmap.md`**:这一版关掉了哪些条目,**当场整条搬去 `docs/roadmap-done.md`**
   —— 不是只加个 ✅。roadmap 只放没做完的事,已完成的原文一字不改地存档(那里面写着
   「当初为什么必须这么排」,是判断依据不是流水账)。只标 ✅ 不搬,文件就会一路长回
   2026-08-03 之前那个样子:114 条里 68 条是完成项,每次盘点都要重付一遍阅读成本。
   roadmap 的条目多是「无 X」这类**否定式能力声明**,功能做了它不会自己变 ——
   代码删个函数会编译报错,文档说"没有"而实际有了,什么都不会响。发版是唯一
   每次都会走完的流程,所以执行点放在这里,不放在记性里。
   (2026-07-29 一次盘点发现 7 处过期,最刺眼的是深色主题当天发布、
   roadmap 还写着「无 dark mode」。)
6. 提交 → `git push origin main` → `git tag vX.Y.Z && git push origin vX.Y.Z`
   → **CI 自动产出全部 7 个产物**(约 10–15 分钟)。
7. `gh run watch` 等 CI 绿;`gh release view vX.Y.Z` 确认 4 个 asset 都在。

   > **CI 先建草稿 release,最后才发布。** 各 job 并行往草稿上挂产物,末尾的
   > `publish` job 把它翻成正式版并设 `--latest`。这样 `/releases/latest` 要么是
   > 上一版、要么是这一版的完整体,不存在「新版本已发布但安装包还没挂上」的窗口
   > —— 应用内更新器正是读这个端点。
   >
   > ⚠️ **`publish` job 挂了的话,release 会停在草稿状态**:产物都在、没有任何东西
   > 被破坏,但用户看不到更新。恢复是一条命令:
   > ```bash
   > gh release edit vX.Y.Z --draft=false --latest
   > ```
8. ~~**带数据改动就先落还原点**~~ —— **2026-08-26 起 playbook 自己做,这一步不用你记了。**

   它拿节点 `.env` 里的 `MICA_VERSION`(当前上线的版本)和要发的 tag 比一次
   `git diff --name-only v<prev>..v<new> -- migrations/`:**有新迁移才**落
   `/data/mica/pre-<version>-<时间戳>.sql.gz`,没有就跳过并说明。**问不出答案时**
   (tag 不在这份 checkout 里、浅克隆)按**有**处理 —— 猜错一次的代价是一点节点 IO,
   猜错另一次的代价是数据库。

   为什么非做不可:迁移是 `sqlx::migrate!` **编译期**嵌进 api 二进制的,启动即执行 ——
   **部署那个镜像就是执行它的迁移**,中间没有可以停下来的一步。而 backup sidecar 是周期
   导出器、且在 api 起来之后才刷,当不了回滚点。

   校验不是只看 `gzip -t`(**实测**:失败的 dump 会留下一个完全合法的、空的 gzip,
   `gzip -t` 照过;截断的也照过)。判据是 **pg_dump 的结尾标记 + `COPY public.document_yrs_base`
   同时在内**,一趟 awk 扫完。另外 `pg_dump | gzip` 那条管线**必须** `set -o pipefail`
   —— 不带的话 dump 失败整条管线仍然返回 0(实测 `rc=0`)。任何一步不过就删掉半成品并
   **中止部署**:没有还原点就不迁移,这才是这一步的意义。

   手动落一次(比如你想在无迁移的发布上也留个点):
   ```bash
   ssh root@mica.cloudcele.com \
     'docker exec mica-postgres-1 pg_dump -U mica -d mica | gzip > /data/mica/pre-X.Y.Z-$(date +%Y%m%d-%H%M%S).sql.gz'
   ```
9. **部署**。两条路**跑的是同一个 playbook**(`ansible/deploy.yml`):

   ```bash
   # 常规:触发即部署(没有审批门 —— 触发本身就是那个决定)
   gh workflow run Deploy --repo weironz/mica -f version=X.Y.Z

   # 兜底(GitHub 挂了)。CI 那一步字面上就是在跑这个脚本
   just deploy-prod X.Y.Z

   # 想先看它要改什么:对真节点跑一遍,不动任何东西
   just deploy-prod X.Y.Z --check --diff
   ```

   顺序:拒绝坏输入 → 从 tag 发 compose → `.env` 写 `MICA_VERSION=vX.Y.Z` → pull
   api/web(**只** app 服务,节点连不上 Docker Hub)→ 确认数据面健康(**只查不动**)
   → 重建 api/web 并等健康 → 刷新已存在的 backup sidecar。
   **任何一步失败都会复原 `.env` 并把上一版拉起来**,所以不会留下「容器还在跑旧版、
   但 `.env` 指着一个不存在的 tag」这种重启即挂的状态。

   > **「compose 改了必须走 deploy-prod」这条已经作废**(2026-08-25)。compose 现在每次
   > 从 tag 发过去,节点不再自存一份可比对的副本 —— 详见上面「compose 不一致这个问题
   > 是怎么消失的」。两条路完全等价,选哪条只取决于 GitHub 在不在。

10. `just verify-prod X.Y.Z` **验证 `/api/health` 真的报这个版本**(workflow 里已内置
   这一步)。
11. **冒烟测这一版真正改了什么。** `verify-prod` 只断言版本号,证明不了功能。挑一个
   改动前后行为可区分的操作实测 —— 例如 v0.12.7 修的是「文档读取 400」,判据就是
   同一个 `mica_read_document` 调用:部署前报 `bad request: block not found:`,
   部署后返回正文。有这种硬判据就用它,没有就手工点一遍受影响的入口。

> `deploy-prod` 会把 `MICA_VERSION` **写进节点 `.env`**,所以之后重启/重启机器都会回到
> 同一个版本,不会悄悄退回旧版。

## 本地还需要什么

**Docker Desktop 仍然必需**(它不是只为发版而装):

- `just dev` —— 本地开发全栈(postgres + rustfs + api + web)
- 容器专属 bug(如 loopback 绑定、Dockerfile 坏掉)由 CI 的 `container` job 抓 —— 它
  构建 api 镜像并把单机栈起起来验 `/api/ready`。以前这是 `just parity-check`,手动且
  可选,所以几乎没人跑
- `just docker-build` / `docker-push` —— **CI 挂掉时的兜底**,正常发版用不到

**部署不需要本地装任何东西** —— 走 `gh workflow run Deploy`,ansible 装在 CI 的 runner 上。

只有兜底 `just deploy-prod` 用得到本地 ansible,而 **ansible 只支持 Linux/macOS 作控制端**
(Windows 上装得上、跑不了)。所以这条路要在 Linux、macOS 或任意 WSL 发行版里跑:

```bash
pipx install ansible-core
ansible-galaxy collection install -r ansible/requirements.yml   # community.docker
```

`deploy-prod` 两样缺一就**拒绝并说清楚**,不会跑到一半才发现 —— 而且第一样检查的是
**能不能运行**,不是文件在不在。**节点侧零新增依赖** —— `docker_compose_v2` 直接驱动
docker CLI,不需要 Python docker SDK。

前置:
```bash
winget install Casey.Just          # just 1.56+,所有 recipe 的入口
choco install innosetup -y         # 仅本地 build-installer 需要(CI 自己装)
```

## 本地手动构建(不走 CI 时)

```bash
just build-cli              # → target/release/mica-cli.exe
just build-installer 0.5.0  # → clients/mica_flutter/installer/Output/Mica-Setup-0.5.0.exe
just build-web              # → deploy/web
just build-api              # → target/release/mica-api-server
just build-all              # 上面除 installer 外全部

just docker-build 0.5.1     # 三个镜像(CI 兜底)
just docker-push 0.5.1      # 需先 docker login registry.cn-shenzhen.aliyuncs.com
```

## 生产环境事实

- **节点**:`root@mica.cloudcele.com`,`/data/mica`,容器 `mica-api-1` / `mica-web-1`。
- **`.env` 两个关键变量**:
  - `MICA_REGISTRY=registry.cn-shenzhen.aliyuncs.com/willspace`(compose 默认值也是它)
  - `MICA_VERSION=vX.Y.Z`(由 `just deploy-prod` 改写)

两份模板的 `MICA_VERSION` **都不用跟着发版改** —— 故意留空,逼使用者自己选一个真实发布。
2026-08-05 它一度钉着 `v0.13.6`(落后 9 个版本),照文档走的人装到的就是那一版;留空让这里
没有会烂的东西。(2026-08-07:`deploy/.env.prod.example` **一份**服务两套栈,默认值以单机为准
—— `SERVER_IP` 打开、`DOMAIN`/`S3_DOMAIN` 注释掉。当天曾拆成两份又合回来:拆开会把
凭据/邮件/注册那 60 行完全相同的说明复制一遍,而那正是本仓库反复付代价的漂移。)

> **两套 compose 现在都用 `${MICA_VERSION:?}`**,空值一律拒绝解析并说明原因。
> 2026-08-07 之前 `docker-compose.yml` 用的是 `${MICA_VERSION:-v0.5.0}` —— 空值**静默**
> 装上 v0.5.0,而模板出厂就是空的。生产节点当时不受影响(`.env` 里的值由 `deploy-prod` 写死),
> 受影响的是照文档装新机的人。
>
> ~~⚠️ 这次改动动了 `deploy/docker-compose.yml`,所以下一次上线必须走 `just deploy-prod`~~
> —— **2026-08-25 起不再需要**:两条路都从 tag 发 compose,`gh workflow run Deploy`
> 和 `just deploy-prod` 完全等价。当时那条限制来自节点自存 compose + CI 只传 sha256
> 指纹的设计,那个设计连同它的代价一起拆掉了(见上面「CI 拿的是 root key」)。
- **节点必须能 pull ACR**:仓库设为公开,或在节点上 `docker login registry.cn-shenzhen.aliyuncs.com`
  一次(凭据只存在节点本地)。
- **`--no-deps`**:只重建 api + web + backup,postgres / rustfs 不动。
- **backup 跟着一起滚**:backup sidecar(mica-cli)和 api/web keyed 同一个 `MICA_VERSION`
  (CI 三个镜像同 tag 一起推 ACR),`deploy-prod` 会在它已运行的节点上一并 `--profile backup pull
  + up -d`,避免像早先那样停在旧 `willdockerhub/mica-cli:v0.3` 漂移。**只刷已在跑 backup 的
  节点**(`ps -aq backup` 探测),不会把没开备份的节点意外打开。首次接这条改动时,对 backup
  还停在旧镜像的节点**重跑一次 `just deploy-prod <当前版本>`** 即可让它追上(api/web 幂等无副作用)。
- **验证不能只看 200**:`just verify-prod X.Y.Z` 会断言 `/api/health` 报的 version 就是
  你要的那个 —— 这是唯一能抓到"镜像没真正更新 / 拉到旧层"的手段。
- **健康版本对了 ≠ 功能对了**:`verify-prod` 只查 version。这一版**真正改了什么**要单独冒烟——
  过一遍本次发版触及的端点/功能。例:v0.11.0 加了 `GET /api/workspaces/export.zip`
  (设置→数据→导出全部工作区),部署后实际点一次、确认下回来的是个含各 workspace 子目录 +
  `workspaces.json` 的 zip;客户端侧改动(工作区上移/下移、文件夹导入容器名)靠桌面 CI 出的
  新安装包,和 prod 部署无关。
- **迁移随 api 镜像自动上**:`crates/infra/src/db.rs` 的 `sqlx::migrate!("../../migrations")`
  在**编译期**把 `migrations/*.sql` 内嵌进 api 二进制,启动时 `run_migrations` 顺序跑。所以
  部署新 api 镜像 = 自动应用新迁移,**不用手动 psql**。两个注意:① 只新增迁移文件、infra 没别的
  改动时,增量编译**可能不重跑** `migrate!` 宏 → `touch crates/infra/src/db.rs` 逼它重编再 build;
  ② 部署前想知道这版带不带迁移,`git diff <上个 tag>..HEAD -- migrations/` 看有没有新文件
  (本次 v0.11.0 **无新迁移**,排序用的 0010 已随 v0.10.0 上线)。

## CI 需要的 secret

仓库级(值不入库、不进任何文档):

| Secret | 用途 |
|---|---|
| `ACR_USERNAME` / `ACR_PASSWORD` | 推阿里云 ACR(用 ACR 的**镜像仓库登录密码**或只授 ACR 权限的 RAM 子账号,**别用账号级 AK/SK**) |
| `DOCKERHUB_USERNAME` / `DOCKERHUB_TOKEN` | 推 Docker Hub(Personal access token,Read & Write) |
| `DEPLOY_SSH_KEY` | 部署用私钥,**2026-08-25 起是有 shell 的 root key**(取舍见上面「CI 拿的是 root key」)。本机对应文件 `~/.ssh/mica-deploy-ci`,节点上是 root 的 `authorized_keys` 里注释为 `github-actions-deploy-mica` 那一行 |
| `DEPLOY_KNOWN_HOSTS` | 节点主机公钥,**钉死**而不是运行时 `ssh-keyscan`(当场扫等于信任任何应答的人,那不叫验证) |

**轮换 / 重建这把 key**(节点侧 + 仓库侧):

```bash
ssh-keygen -t ed25519 -f ~/.ssh/mica-deploy-ci -N "" -C github-actions-deploy-mica
# 注意是 "restrict 空格 ssh-ed25519",不是逗号 —— 见下面那条
printf 'restrict %s\n' "$(cat ~/.ssh/mica-deploy-ci.pub)" \
  | ssh root@mica.cloudcele.com 'sed -i "/github-actions-deploy-mica/d" ~/.ssh/authorized_keys; cat >> ~/.ssh/authorized_keys'
gh secret set DEPLOY_SSH_KEY --repo weironz/mica < ~/.ssh/mica-deploy-ci   # 从文件读,不进 history
```

这把 key 带 **`restrict`** 选项:能以 root 执行命令,但**拿不到 PTY、开不了端口转发**
(实测:`tty` 报 `PTY allocation request failed`;`-R` 报 `remote port forwarding
failed`)。这是拆掉 `command=` 栅栏之后还能白拿的一片 —— 它挡不住「以 root 跑任意命令」,
但挡住了把这条连接当跳板往内网转发(比如把 postgres 的 5432 转出去)。Ansible 不 `become`
时不需要 PTY,实测带 `restrict` 跑完整个 playbook 与不带完全一致。

⚠️ **`authorized_keys` 的选项和 key 之间是空格,选项彼此之间才是逗号。** 写成
`restrict,ssh-ed25519 AAAA…` 的话,sshd 会把 `ssh-ed25519` 当成第二个**选项名**,
不认识 → **整行作废**,认证失败且默认日志级别下**不给任何理由**。装完一定要用
`ssh -F NUL -i <key> -o IdentitiesOnly=yes` 真连一次验证 ——
注意 `IdentitiesOnly=yes` **仍然允许默认身份文件**(`~/.ssh/id_*`),所以不加
`-F NUL`(或 `-F /dev/null`)的话,顶上来的是你本机原来那把 key,测了等于没测。

设置方式(值不会留在 shell history):
```bash
gh secret set ACR_USERNAME
gh secret set ACR_PASSWORD
gh secret set DOCKERHUB_USERNAME
gh secret set DOCKERHUB_TOKEN
```

> 凭据文件(`password.txt` / `aliyun-ak.txt` 等)已在 `.gitignore` 里。这是个**公开仓库**,
> 镜像仓库的写权限一旦外泄,别人能往生产镜像里推任意内容,节点会照单全收地拉下来跑。

## 坑(踩过的)

- **Windows 上 PATH 里的 `bash` 是 WSL 的**(`C:\WINDOWS\system32\bash.exe`)。justfile 用
  `set windows-shell` 钉死 Git Bash;走 WSL 的话那边没有 Windows 的 docker/flutter/cargo,
  路径还变 `/mnt/d/...` → 全挂。
- **`just deploy-prod` 从 PowerShell 跑报 `could not find cygpath ... shebang interpreter`**:
  `deploy-prod` 是 shebang recipe(`#!/usr/bin/env bash`),`just` 对 shebang recipe **不走**
  `set windows-shell`,而是直接执行解释器,并用 `cygpath` 把临时脚本路径翻成 Unix 风格。
  `cygpath.exe` 在 `C:\Program Files\Git\usr\bin\`,但 PowerShell 的 PATH 通常只有 `Git\bin`
  → 找不到。修法二选一:(A) **从 Git Bash 里跑**(那里 cygpath 在 `/usr/bin`);(B) PowerShell 里
  临时挂 PATH:`$env:PATH = "C:\Program Files\Git\usr\bin;$env:PATH"` 再 `just deploy-prod X.Y.Z`。
- **Windows 没有 `rsync`**,暂存 bundle 用 `rm -rf` + `cp -r`。
- **`docker build` 必须带 `--provenance=false --sbom=false`**(CI 里是 build-push-action 的
  `provenance: false` / `sbom: false`)。buildx 默认挂 OCI attestation,镜像变成多 manifest
  索引,部分仓库和 `docker load` 解不开。
- **Inno Setup 不在默认环境里**,本地 `build-installer` 前先 `choco install innosetup -y`。
- Docker Desktop 没启动时 `docker build` 报 `npipe:////./pipe/dockerDesktopLinuxEngine` 找不到。

## 相关文档

- `docs/dev-environment.md` —— 换机重配(MCP / 工具链 / Windows 构建前置)
- `docs/desktop-plan.md` —— 桌面端路线与环境备忘
- `docs/backup.md` —— mica-cli 与外部备份
