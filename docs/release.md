# 发版与构建(Release & build)

**一句话**:**CI 构建一切,`just` 决定何时上线。**

推一个 `v*` tag,GitHub Actions 产出全部 7 个产物;之后你跑 `just deploy-prod X.Y.Z`
把生产滚到那个版本。两步,没有第三步。

| 产物 | 谁构建 | 去哪 |
|---|---|---|
| `Mica-Setup-X.Y.Z.exe` | **CI** job `windows` | GitHub Release(驱动应用内自动更新) |
| `mica-cli` ×3(win/linux/macos) | **CI** job `cli` | GitHub Release |
| `mica-api` / `mica-web` / `mica-cli` 镜像 | **CI** job `images` | **阿里云 ACR**(生产拉这里)+ Docker Hub(异地副本) |
| 生产上线 | **你**:`just deploy-prod X.Y.Z` | node72 从 ACR pull |

## 为什么 CI 不做部署

让 CI 能部署,就得把**生产 root SSH key 放进 GitHub secrets** —— 仓库或 Actions 一旦
被攻破,等于生产被攻破。现在 CI 只持有**推镜像的凭据**(权限小、可单独吊销),上线是
你主动按的一下。这个边界是安全上的实质区别,不是流程洁癖。

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

1. **版本号三处同步**(必须一致):
   - `clients/mica_flutter/pubspec.yaml` 的 `version:`
   - `clients/mica_flutter/lib/main.dart` 的 `kAppVersion`
   - `crates/api-server/Cargo.toml` 的 `version`(顺带 `cargo check` 更新 `Cargo.lock`)
2. **判断服务端要不要跟着发**:改动是否触及 `crates/markdown` 等服务端依赖?
   链路 `api-server → mica-app-core → mica-markdown`。用 `cargo tree -p <crate> | grep <dep>`
   实证,别猜。(例:v0.5.0 的 CJK 强调改了 markdown → api 必须重建。)
3. `just test` 全绿。**跑测试时要带 `DATABASE_URL`** —— 否则 `sync_pg` 这类 DB 集成
   测试会**静默跳过**,整套 0.00s「全过」,那是真空通过不是验证(见 `docs/lessons.md`)。
   本地栈起着的话:`$env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"`,
   跑完看耗时 —— 秒级才说明真跑了。想更稳再跑 `just parity-check`(容器形态,见下)。
4. 提交 → `git push origin main` → `git tag vX.Y.Z && git push origin vX.Y.Z`
   → **CI 自动产出全部 7 个产物**(约 10–15 分钟)。
5. `gh run watch` 等 CI 绿;`gh release view vX.Y.Z` 确认 4 个 asset 都在。

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
6. **带数据改动就先落还原点**(`deploy-prod` 自己不做备份,理由见「生产运维要点」):
   ```bash
   ssh root@mica.cloudcele.com \
     'docker exec mica-postgres-1 pg_dump -U mica -d mica | gzip > /data/mica/pre-X.Y.Z-$(date +%Y%m%d-%H%M%S).sql.gz'
   # 验完整性:gzip -t <file>,再 zcat <file> | grep -c "^COPY public.documents " 确认目标表在内
   ```
7. `just deploy-prod X.Y.Z` → 节点改 `.env` 的 `MICA_VERSION` → 从 ACR pull → 重建
   api+web → 等健康 → `just verify-prod X.Y.Z` **验证 `/api/health` 真的报这个版本**。
8. **冒烟测这一版真正改了什么。** `verify-prod` 只断言版本号,证明不了功能。挑一个
   改动前后行为可区分的操作实测 —— 例如 v0.12.7 修的是「文档读取 400」,判据就是
   同一个 `mica_read_document` 调用:部署前报 `bad request: block not found:`,
   部署后返回正文。有这种硬判据就用它,没有就手工点一遍受影响的入口。

> `deploy-prod` 会把 `MICA_VERSION` **写进节点 `.env`**,所以之后重启/重启机器都会回到
> 同一个版本,不会悄悄退回旧版。

## 本地还需要什么

**Docker Desktop 仍然必需**(它不是只为发版而装):

- `just dev` —— 本地开发全栈(postgres + rustfs + api + web)
- `just parity-check` —— 发版前跑**真镜像**,抓容器专属 bug(如 loopback 绑定)
- `just docker-build` / `docker-push` —— **CI 挂掉时的兜底**,正常发版用不到

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
