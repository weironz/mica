# 新开发机从零搭建(Windows)

> **这份文档的定位**:一台干净的 Windows 机器,照着从上往下做完,就能编译后端、跑桌面/web
> 客户端、并让 Claude Code 带全套 MCP 工作。命令都是 2026-07-20 在一台全新 Windows 11 机器上
> **实际跑过并验证**的,不是照着别处抄的。
>
> 需要更深的背景时再去看:MCP/skills 细节与踩坑 → [`dev-environment.md`](dev-environment.md);
> 桌面端里程碑 → [`desktop-plan.md`](desktop-plan.md);发版工具链 → [`release.md`](release.md)。

## 0. 先看这一段

- 全程约 **1.5~2 小时**,绝大部分是等下载。瓶颈是 VS Build Tools 和 Flutter SDK。
- **国内网络必须先配镜像**,否则 Flutter SDK 会从几分钟 60MB 的速度爬(见 §3)。
- 需要**管理员权限**的只有:VS Build Tools 安装、开发者模式开关。其余都是用户级。
- 磁盘预留 **≥ 25GB**(VS Build Tools ~7GB、Flutter SDK 解压后 ~3.5GB、cargo 缓存 + target ~10GB)。

## 1. 系统前置

### 开发者模式(必须)

项目含原生插件(`window_manager` 等),Flutter 在 Windows 上用**符号链接**管理插件,不开会在
`pub get` 阶段直接报 `Building with plugins requires symlink support`。

```powershell
start ms-settings:developers   # 打开「开发人员模式」,普通用户即可,无需重启
```

验证(⚠️ **别用 PowerShell 验证,会假阴性**,原因见 [`dev-environment.md`](dev-environment.md)):

```powershell
# 看开关本身
(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock').AllowDevelopmentWithoutDevLicense   # 应为 1
# 看实际能力(这条才作数)
cmd /c mklink /D C:\temp\__symtest C:\Windows
```

### PowerShell 7(推荐)

```powershell
winget install --id Microsoft.PowerShell --exact --silent --accept-package-agreements --accept-source-agreements
```

> **Windows PowerShell 5.1 不要试图卸载**——它是 Windows 内置系统组件(`System32\WindowsPowerShell\v1.0`),
> 没有卸载入口,大量系统功能和 MSI 自定义动作依赖它。PS7 (`pwsh.exe`) 是**并行安装**,设计上就是共存。
> 5.1 只是别拿它当日常 shell:本文档里几个坑(符号链接假阴性、`Set-Content -Encoding utf8` 写 BOM、
> 原生命令 stderr 被包成 ErrorRecord)全都是 5.1 独有的行为。

## 2. 核心工具链

以下全部走 winget,**包 ID 照抄,别搜名字**(搜 "flutter" 出来的全是第三方 App)。

```powershell
winget install --id Git.Git              --exact --silent --accept-package-agreements --accept-source-agreements
winget install --id GitHub.cli           --exact --silent --accept-package-agreements --accept-source-agreements
winget install --id OpenJS.NodeJS        --exact --silent --accept-package-agreements --accept-source-agreements
winget install --id Rustlang.Rustup      --exact --silent --accept-package-agreements --accept-source-agreements
winget install --id Casey.Just           --exact --silent --accept-package-agreements --accept-source-agreements
winget install --id Docker.DockerDesktop --exact --silent --accept-package-agreements --accept-source-agreements
```

**VS Build Tools**(`flutter build windows` 和 Rust 的 MSVC 链接器都要它;**不需要 VS Community**):

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --silent `
  --accept-package-agreements --accept-source-agreements `
  --override "--wait --quiet --norestart --add Microsoft.VisualStudio.Workload.VCTools --add Microsoft.VisualStudio.Component.Windows11SDK.26100 --includeRecommended"
```

> `--override` 那串是关键。不带 workload 参数装出来的是个空壳,`flutter doctor` 会认到
> Visual Studio 但缺 C++ 组件,`cargo build` 则会拖到链接阶段才失败。

**Flutter SDK** —— winget 没有官方包,手工装。**先看 §3 配好镜像再下**:

```powershell
# 1.8GB。走 storage.flutter-io.cn 实测 122 秒;走 googleapis 是几小时的量级
$url = "https://storage.flutter-io.cn/flutter_infra_release/releases/stable/windows/flutter_windows_3.44.6-stable.zip"
Start-BitsTransfer -Source $url -Destination "$env:TEMP\flutter.zip"
Expand-Archive "$env:TEMP\flutter.zip" -DestinationPath "C:\"        # zip 内含顶层 flutter/ 目录 → C:\flutter
[Environment]::SetEnvironmentVariable("PATH", [Environment]::GetEnvironmentVariable("PATH","User") + ";C:\flutter\bin", "User")
```

装到 **`C:\flutter`**(全项目约定路径)。最新 stable 版本号从
`https://storage.flutter-io.cn/flutter_infra_release/releases/releases_windows.json` 的
`current_release.stable` 查。项目要求 Dart `>=3.8.0 <4.0.0`。

## 3. 国内网络镜像(在装 Flutter 之前设)

```powershell
[Environment]::SetEnvironmentVariable("FLUTTER_STORAGE_BASE_URL", "https://storage.flutter-io.cn", "User")
[Environment]::SetEnvironmentVariable("PUB_HOSTED_URL",           "https://pub.flutter-io.cn",     "User")
```

不设的话 SDK 下载和后续每次 `flutter pub get` 都会卡在同一个问题上。
(清华的 `mirrors.tuna.tsinghua.edu.cn/flutter/` 路径已 404,USTC 返回空,别浪费时间试。)

Rust 侧默认源在国内够用,实测 `cargo fetch` 整个 workspace 无需换源。

## 4. 仓库与配置

```powershell
git clone https://github.com/weironz/mica.git C:\data\codes\mica
cd C:\data\codes\mica
Copy-Item .env.example .env      # 开发用默认值即可;S3_* / ANTHROPIC_API_KEY 不配则对应端点返回 503
cargo fetch                      # 预热依赖缓存,可与其他安装并行
```

`gh` 登录(GitHub MCP 依赖它,见 §5):

```powershell
gh auth login                    # 需要 scopes: repo, read:org, workflow, gist
```

## 5. MCP 与插件

分两层:**项目级**随仓库走(`.mcp.json`,已入库,不用配);**用户级**是机器全局,换机必须重配。

| 服务 | 层级 | 装法 |
|---|---|---|
| playwright | 项目级 `.mcp.json` | 已入库。首次会话要在交互式 `claude` 里**批准一次** |
| codebase-memory-mcp | 用户级 | 见下,**必装** —— CLAUDE.md 要求探索代码优先走图谱 |
| GitHub MCP | 用户级 | 见下,远端 OAuth 方案已坏,必须走本地二进制 |
| context7 | 插件 | 查库/框架文档;`/plugin` 装 |
| chrome-devtools | 插件 | 调试/性能/网络;`/plugin` 装 |

### codebase-memory-mcp(必装)

官方 PowerShell 安装器会自动配好 Claude Code 的 MCP 入口、hooks 和 skill:

```powershell
Invoke-WebRequest -Uri https://raw.githubusercontent.com/DeusData/codebase-memory-mcp/main/install.ps1 -OutFile install.ps1
Unblock-File .\install.ps1
.\install.ps1                    # 装到 ~/.local/bin + AppData\Local\Programs\,用户级,不需要管理员
```

装完**首次建图**(不建的话所有图谱查询都是空的):

```powershell
codebase-memory-mcp cli index_repository --repo-path "C:/data/codes/mica" --mode full
```

项目名由路径推导(如 `C-data-codes-mica`),**之后所有查询工具都要带 `project` 参数**。
`full` 模式偶发 worker 崩溃且崩溃日志是空文件——原样重试一次即可,别降级到 `fast`
(`full` 8979 节点 / 37710 边,`fast` 只有 5773 / 27559)。

### GitHub MCP(远端 OAuth 已坏,走本地二进制)

`claude mcp add --transport http ... https://api.githubcopilot.com/mcp/` 这条路会在 `/mcp`
授权时报 `HTTP 400`。**端点本身是好的**(拿 `gh auth token` 手工 POST `initialize` 返回 200),
坏的是插件那条 OAuth 流程,别在上面反复重连。

```powershell
gh release download --repo github/github-mcp-server --pattern "github-mcp-server_Windows_x86_64.zip" --dir $env:TEMP
Expand-Archive "$env:TEMP\github-mcp-server_Windows_x86_64.zip" -DestinationPath "$env:TEMP\ghmcp" -Force
Copy-Item "$env:TEMP\ghmcp\github-mcp-server.exe" "$env:USERPROFILE\.local\bin\" -Force
claude mcp add --scope user github -- cmd /c "$env:USERPROFILE\.local\bin\github-mcp.cmd"
```

包装脚本 `~/.local/bin/github-mcp.cmd` —— 启动时现从 `gh` 取令牌,**凭证不落任何配置文件**,
且 gh 轮换令牌后自动跟随:

```bat
@echo off
setlocal
for /f "delims=" %%i in ('gh auth token') do set "GITHUB_PERSONAL_ACCESS_TOKEN=%%i"
if not defined GITHUB_PERSONAL_ACCESS_TOKEN (
  echo github-mcp: could not get a token from 'gh auth token' - run 'gh auth login' 1>&2
  exit /b 1
)
"%~dp0github-mcp-server.exe" stdio
```

### Skills

见 [`dev-environment.md`](dev-environment.md) 的 Skills 一节 —— 注意 `npx skills add` 装到
`~/.agents/skills/` 而 **Claude Code 只读 `~/.claude/skills/`**,装完要手工复制过去。

## 6. 验收

四条全过才算搭好。任何一条不过,**别往下做业务开发**,先回头补:

```powershell
cargo build --workspace     # 全 8 个 crate。这条同时验证 MSVC 链接器接上了(实测 8m25s)
flutter doctor -v           # Flutter / Windows Version / VS Build Tools / 设备 应全绿
just --list                 # 验证 just 能读到 justfile(18 条 recipe)
cargo test                  # 可选但推荐
```

`flutter doctor` 里 **Android toolchain 报 ✗ 是正常的** —— 本项目只覆盖 Windows 和 Web,
不需要 Android SDK。其余项必须绿,尤其 `Visual Studio - develop Windows apps`。

起本地后端:

```powershell
just dev-up                 # Docker 里的 postgres + rustfs
just dev-api                # cargo run -p mica-api-server(启动即跑迁移)
just app                    # Flutter 桌面客户端;just app chrome 跑 web
```

## 7. 实测耗时参考

一台 16GB 内存的 Windows 11 笔记本,家用宽带 + 国内镜像:

| 步骤 | 耗时 |
|---|---|
| Rust + just(winget) | ~2 分钟 |
| VS Build Tools | ~25 分钟 |
| Flutter SDK 下载(镜像)| 122 秒 / 1.8GB / 14.86 MB/s |
| Flutter SDK 解压 | 123 秒 |
| `flutter --version` 首跑(编译 flutter tool)| ~3 分钟 |
| `cargo fetch` | ~4 分钟 |
| `cargo build --workspace` | 8 分 25 秒 |
| codebase-memory-mcp 建图(`full`)| < 1 分钟 |

## 8. 这轮踩到的坑速查

| 现象 | 真因 | 对策 |
|---|---|---|
| 开发者模式开了,建符号链接仍报要管理员 | PS 5.1 的 `New-Item -ItemType SymbolicLink` 不传 `ALLOW_UNPRIVILEGED_CREATE` | 用 `mklink` 验证;是假阴性 |
| Flutter SDK 下载龟速 | 默认源 `storage.googleapis.com` | 换 `storage.flutter-io.cn`,快 ~50 倍 |
| GitHub MCP `HTTP 400` | 插件的远端 OAuth 流程没拿到凭证(端点本身正常) | 换本地二进制 + `gh auth token` |
| 图谱查询报 `missing required argument: project` | 查询工具都要带 `project` | 带上项目名(由仓库路径推导) |
| 建图 `exit_nonzero`,worker 日志是空文件 | 偶发崩溃,查不出触发文件 | 原样重试一次 |
| winget 装完 VS Build Tools 但 `flutter doctor` 缺 C++ | 没带 `--override` 指定 workload | 用 §2 的完整命令重装 |
| git 提交标题开头有不可见字符 | PS 5.1 `Set-Content -Encoding utf8` 写 BOM | 用 `git commit -m @'...'@` here-string,别过文件 |
