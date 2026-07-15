# 发版与构建(Release & build)

一次发版产出 **5 样东西**,分两条流水线。**记住这条边界**:

| 产物 | 谁构建 | 怎么触发 |
|---|---|---|
| `Mica-Setup-X.Y.Z.exe`(Windows 桌面安装包) | **GitHub Actions** | 推 `v*` tag |
| `mica-cli-X.Y.Z-{windows-x64.exe,linux-x64,macos-arm64}` | **GitHub Actions** | 推 `v*` tag(同上) |
| `mica-web` 镜像 → 线上 web | **本地手动** | `just deploy-prod` |
| `mica-api` 镜像 → 线上 API | **本地手动** | `just deploy-prod`(同上) |
| Docker Hub 副本(api + web) | **本地手动** | `just docker-push`(含在 `deploy-prod` 里) |

> **Actions 管"发给用户的东西"(安装包 + CLI);"线上服务"(web + api)全靠本地手动。**
> web/api 没有任何 CI —— 别指望推了 tag 线上就更新了。

## GitHub Actions

两个 workflow,都在 `.github/workflows/`:

- **`release-windows.yml`(Release)** —— 触发:推 `v*` tag(或手动 `workflow_dispatch`,`publish=false` 时为 dry-run)。
  - job `windows`:`flutter build windows --release` → Inno Setup 打包 → 安装包 attach 到 GitHub Release。
  - job `cli`:3 平台矩阵,`cargo build --locked -p mica-cli --release` → attach 到同一个 Release。
  - **`cli` 特意 `needs: windows`**:应用内自动更新读 GitHub 的 `/releases/latest`,不能让它看到一个还没挂上安装包的 release。同理 `prerelease: false` + `make_latest: true` —— 标成 prerelease 会让自动更新完全看不见新版本。
- **`ci.yml`(CI)** —— 触发:推 main / PR。只跑测试(cargo build + `mica-markdown`/`mica-core`/`mica-interchange` 测试 + flutter analyze/test)。**不产出任何产物。**

## 前置条件(本地)

```bash
winget install Casey.Just          # just 1.56+,所有 recipe 的入口
choco install innosetup -y         # 仅 build-installer 需要
docker login                       # 仅 docker-push 需要
```
Flutter / Rust / Docker Desktop / OpenSSH(连 node)见 `docs/dev-environment.md`。

## 本地手动构建四个产物

`just --list` 看全部。四个产物各一条:

```bash
just build-cli              # 1/4  → target/release/mica-cli.exe
just build-installer 0.5.0  # 2/4  → clients/mica_flutter/installer/Output/Mica-Setup-0.5.0.exe
just build-web              # 3/4  → deploy/web(顺带打印 bundle md5)
just build-api              # 4/4  → target/release/mica-api-server(本地跑/剖析用)
just build-all              # 1+3+4(installer 要 Windows 专属工具链,不含在内)
```

镜像与推送:

```bash
just docker-build           # 构建 mica-api + mica-web 两个镜像(会先跑 build-web)
just docker-push            # 推 Docker Hub
```

## 完整发版流程

1. **版本号三处同步**(必须一致):
   - `clients/mica_flutter/pubspec.yaml` 的 `version:`
   - `clients/mica_flutter/lib/main.dart` 的 `kAppVersion`
   - `crates/api-server/Cargo.toml` 的 `version`(顺带 `cargo check` 更新 `Cargo.lock`)
2. **判断要不要重建 api**:改动是否触及 `crates/markdown` 等服务端依赖?
   链路是 `api-server → mica-app-core → mica-markdown`。用 `cargo tree -p <crate> | grep <dep>` 实证,别猜。
   (例:v0.5.0 的 CJK 强调改了 markdown → api 必须重建;`mica-cli` 不依赖 markdown → 其镜像不用动。)
3. `just test` 全绿。
4. 提交 → `git push origin main` → `git tag vX.Y.Z && git push origin vX.Y.Z`
   → **Actions 自动出安装包 + 3 个 CLI 二进制**。
5. `just deploy-prod` → 构建镜像 → 送上 node → 重建服务 → 推 Hub → 验证。
6. `gh run watch` 看 Actions;`gh release view vX.Y.Z` 确认 4 个 asset 都在。

纯客户端发版(没碰服务端)可以只 `just deploy-prod` 发 web、跳过 api——但 `deploy-prod` 是两个一起来,想只发 web 就手动 `just docker-build && just ship`,或直接接受 api 重建一次(无害)。

## 线上部署的关键事实

- **节点**:`root@mica.cloudcele.com`,`/data/mica`,容器名 `mica-api-1` / `mica-web-1`。
- **节点连不上 Docker Hub** —— 所以镜像走 `docker save | gzip | ssh docker load`(`just ship`),**不是** `docker pull`。推 Hub 只是异地副本,不是投递路径。
- **滚动标签 `v0.3` ≠ 应用版本**。compose 读 `/data/mica/.env` 的 `MICA_VERSION`,这个指针从 v0.3 一路滚到了 v0.5.x。要改名得同时改节点 `.env` **并**重打 `mica-cli` 镜像(backup 服务用它),否则那个服务会去 Hub 拉取然后失败。
- **`--no-deps`**:只重建 api + web,postgres / rustfs / backup 不动。
- **验证不能只看 200**:`just verify-prod` 会比对**线上 `main.dart.js` 的 md5 与本地 `deploy/web/` 的是否一致** —— 这是唯一能抓到"镜像陈旧 / 层缓存没更新"的手段。再加 `/api/health` 的 version 与 `/mcp` 状态码。

## 坑(踩过的)

- **Windows 上的 `bash` 是 WSL 的**(`C:\WINDOWS\system32\bash.exe`)。justfile 用 `set windows-shell` 钉死 Git Bash;若走 WSL,那边没有 Windows 的 docker/flutter/cargo,路径还变成 `/mnt/d/...` → 全挂。
- **Windows 没有 `rsync`**。旧 recipe 用 rsync 暂存 bundle,在 Windows 上直接失败;现用 `rm -rf` + `cp -r`。
- **`docker build` 必须带 `--provenance=false --sbom=false`**。buildx 默认挂 OCI attestation,镜像变成多 manifest 索引,节点上 `docker load` 解不开。
- **Inno Setup 不在默认环境里**,`build-installer` 前先 `choco install innosetup -y`。
- Docker Desktop 没启动时 `docker build` 报 `npipe:////./pipe/dockerDesktopLinuxEngine` 找不到 —— 启动它再来。

## 相关文档

- `docs/dev-environment.md` —— 换机重配(MCP / 工具链 / Windows 构建前置)
- `docs/desktop-plan.md` —— 桌面端路线与环境备忘
- `docs/backup.md` —— mica-cli 与外部备份
