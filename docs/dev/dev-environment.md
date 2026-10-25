# Mica 开发环境与辅助工具

> **新机器从零搭建请先看 [`bootstrap.md`](bootstrap.md)** —— 那份是按顺序照做的完整清单(系统前置、
> 工具链、国内镜像、MCP、验收、实测耗时)。**本文是参考层**:同样的东西讲得更细,并记录每条规则的来历
> 和踩过的坑。两份有出入时以 `bootstrap.md` 的命令为准(它是实测验证过的)。
>
> 另见:发版/构建工具链(Inno Setup、docker login)→ `docs/ops/release.md`;桌面端里程碑 → `docs/plans/desktop-plan.md`。

## MCP 服务器

分两层:**项目级**随仓库走(`.mcp.json`,提交进库);**用户级**是机器全局,换机要各自重配。

### 项目级(`.mcp.json`,已入库)

| 服务 | 命令 | 备注 |
|---|---|---|
| playwright | `npx -y @playwright/mcp@latest --headless` | web 端实测截图 |

### 用户级(全局配置,换机单独重配)

这些**不在仓库**里,配在 Claude Code 用户级(`claude mcp add ...` 或用户 settings):

- **code-review-graph** — 代码知识图谱(见下节；根 `AGENTS.md` 要求探索代码优先用它)
- **GitHub MCP** — 仓库/PR/issue 操作
- **Playwright MCP** — 浏览器自动化(与项目级重复,按需)
- **Chrome DevTools MCP** — 调试/性能/网络
- **Context7 MCP** — 实时拉取库/框架文档(全局规则要求查库文档时优先用它)

> 迁移时用 `claude mcp list` 导出现有配置作为权威来源,本表仅作清单提醒。

## code-review-graph（知识图谱）

根 `AGENTS.md` 要求探索代码**优先用 code-review-graph**。用户级 MCP 配置以 Codex 的 `~/.codex/config.toml`、Claude Code 的 `~/.claude.json` 为准；全局启动命令为 `code-review-graph serve`，不要将其固定到另一台机器或某个项目目录。

工具面：`semantic_search_nodes` 找符号，`query_graph` 查调用/导入/测试关系，`get_architecture_overview` / `list_communities` 看结构，`detect_changes` + `get_review_context` 审改动，`get_impact_radius` / `get_affected_flows` 分析影响；具体参数以当前 MCP 暴露的 schema 为准。

在仓库根运行 `code-review-graph status` 检查状态；初次用 `build`，后续用 `update` 增量刷新。图谱保存在被 Git 忽略的 `.code-review-graph/`，查询其他仓库时显式传 `repo_root`。

Git/编辑 hooks 可以调用 CLI 自动更新，但 hooks 是本机配置，不随仓库迁移。新机器须分别验证 MCP 连接、CLI 和 hooks；MCP 尚未加载时可用 CLI，不能仅凭配置文件声称工具已生效。

## Skills

```bash
# Flutter / Dart 官方 skills
npx skills add flutter/skills --skill '*' --agent claude-code --yes
npx skills add dart-lang/skills --skill '*' --agent claude-code --yes

# Karpathy 编码守则插件市场
/plugin marketplace add forrestchang/andrej-karpathy-skills
```

> ⚠️ **坑(实测 2026-06-06,Windows + Claude Code 2.1.x)**:`npx skills add` 会把
> skills 装到 `~/.agents/skills/`(或 `--agent universal` 时散写进各 IDE 目录),
> **Claude Code 并不读这里**——它只读 `~/.claude/skills/`(全局)和项目
> `.claude/skills/`。所以装完要把 skill 目录**复制进** `~/.claude/skills/` 才能被
> Skill 工具调用,例如:
> `Copy-Item ~/.agents/skills/flutter-* ~/.claude/skills/ -Recurse -Force`
> (复制后下次会话生效)。`--agent universal` 还会在仓库根撒一堆 `AGENTS.md`/
> `.cursorrules`/`.gemini/` 等,已 gitignore(见 `.gitignore` 末尾)。

## GitHub MCP(用户级,本地二进制 + gh token)

⚠️ **远端 OAuth 方案已不可用**:`claude mcp add --transport http --scope user github
https://api.githubcopilot.com/mcp/` 加完后 `/mcp` 授权失败,报
`HTTP 400 at https://api.githubcopilot.com/mcp/`。**端点本身是好的**——拿 `gh auth token`
的令牌手工 POST 一个 `initialize` 会返回 200,所以问题出在插件那条 OAuth 流程没拿到有效凭证,
不是网络或服务端。别在这上面反复重连。

改用**官方本地 server + 启动时现取令牌**(2026-07-20 实测可用):

```powershell
# 1. 装二进制(github/github-mcp-server,认准 Windows_x86_64)
gh release download --repo github/github-mcp-server --pattern "github-mcp-server_Windows_x86_64.zip"
# 解压后把 github-mcp-server.exe 放到 ~/.local/bin/

# 2. 注册(包装脚本见下)
claude mcp add --scope user github -- cmd /c "C:\Users\willz\.local\bin\github-mcp.cmd"
```

包装脚本 `~/.local/bin/github-mcp.cmd` 在启动时从 `gh auth token` 取令牌塞进
`GITHUB_PERSONAL_ACCESS_TOKEN`,**令牌不落任何配置文件**,且 gh 轮换令牌后自动跟随:

```bat
@echo off
setlocal
for /f "delims=" %%i in ('gh auth token') do set "GITHUB_PERSONAL_ACCESS_TOKEN=%%i"
"%~dp0github-mcp-server.exe" stdio
```

本机 `gh` CLI 已登录(`weironz`,scopes:`gist` / `read:org` / `repo` / `workflow`)。

> 插件版 `plugin:github:github` 仍会在 `claude mcp list` 里显示 `Failed to connect`——
> 那是同一个坏掉的远端 OAuth 入口,和上面这个用户级 `github` 是两条独立配置。要么在
> `/plugin` 里禁掉它,要么忽略这行噪音。

## 工具链版本:钉死的,不是「最新」

两个文件说了算,CI 和本机读的是同一个号:

| 钉在哪 | 管什么 | 谁读它 |
| --- | --- | --- |
| `.fvmrc` | Flutter(Dart 随它一起) | 六个 `subosito/flutter-action` step 的 `flutter-version-file` |
| `rust-toolchain.toml` | Rust | rustup(仓库内每次 `cargo`/`rustc` 自动生效) |
| `deploy/Dockerfile.api`·`.cli`·`docker-compose.yml` 的 `rust:<版本>-slim-bookworm` | **出厂镜像**的 Rust | 三处必须与上一行同号 |

**`just release` 会拒绝**本机 Flutter ≠ `.fvmrc`、本机 rustc ≠ `rust-toolchain.toml`、
或三个镜像与钉子不同号。所以这不是「记得对齐」,是过不去。

**升级的做法**(一次提交,不是某天自己发生):

```bash
flutter upgrade                    # 或 checkout 指定版本
# 改 .fvmrc / rust-toolchain.toml / 三个 rust:<版本> 标签，改成同一个号
just release-check                 # 它会指出还有哪一处没跟上
```

> **为什么钉**:此前全都是 `channel: stable`,也就是「构建那天 rustup / flutter-action
> 解析出什么就是什么」。于是 CI 一个版本、笔记本另一个、出厂镜像第三个,而且没有任何记录。
> Flutter 3.47 就是这样**在没人拍板的情况下把 Windows 默认渲染器换成 Impeller 并发给了
> 用户**(0.13.29 就是这么构建的),而本机还停在 3.44 —— 于是 `just app` 调的是一个我们
> 根本不发布的引擎。代价照实说:钉死之后**升级要人主动做**,不会自己跟上游。

## Windows 桌面构建前置

- **Flutter SDK**(本机 `C:\flutter`,已入 PATH;版本由 `.fvmrc` 钉死,见上)、**VS Build Tools + Desktop C++ workload + Win10 SDK**(`flutter build windows` 需要)。
- **开发者模式(Developer Mode)**:一旦项目含**任何原生插件**(如 `window_manager`),Flutter 在 Windows 上用符号链接管理插件,需开开发者模式,否则 `pub get`/构建报 `Building with plugins requires symlink support`。开法:`start ms-settings:developers` → 打开「开发人员模式」(普通用户即可,无需重启)。M1 无插件时不需要,M2 起需要。

  ⚠️ **别用 PowerShell 的 `New-Item -ItemType SymbolicLink` 验证是否生效**:Windows PowerShell 5.1 建符号链接时不传 `SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE`,即使开发者模式已开也会报 `Administrator privilege required`,是**假阴性**。用 `cmd /c mklink /D <链接> <目标>` 验证才准(Flutter 走的也是带该标志的 Win32 API)。真正的开关状态查注册表:`HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock` 的 `AllowDevelopmentWithoutDevLicense` == 1。
- **Docker Desktop**:跑本地后端(端口原生发布到 Windows localhost)。

## 重配顺序(换机)

1. 核心工具链:Flutter、Rust、Docker Desktop、平台 SDK + **开发者模式**(见上「Windows 桌面构建前置」)。
2. 本文的 MCP（项目级配置见 `.mcp.json`，用户级含 code-review-graph）、首次 `code-review-graph build` 建图、skills。
3. 起后端:`docker compose up -d --build postgres api`(详见 `docs/plans/desktop-plan.md` M1 状态)。

## 跑集成测试(尤其是云同步)

`integration_test/` 分两类,前置条件完全不同:

**不需要后端** —— `cloud_sync_integrity_test.dart` 自带一个**进程内假 WS 服务端**
(`_FakeSyncServer`),纯客户端就能跑同步逻辑:

```sh
cd clients/mica_flutter
flutter test integration_test/cloud_sync_integrity_test.dart -d windows
```

改同步代码时优先复用这个骨架——比起全栈 e2e,它快且不依赖 docker。

**需要真后端** —— `migration_sync` / `cloud_sync` / `page_switch_fidelity` /
`offline_image_reconcile` 要整个栈起着:

```sh
docker compose up -d postgres rustfs api    # 账号 demo@mica.dev / password123
```

⚠️ **两个测试文件不要一起跑**:会撞 debug-connection race。分开跑,或者中间
`kill mica_flutter` 再 sleep 一下。

⚠️ **两个引擎变体必须同步改**:`cloud_sync_io.dart`(桌面走 FFI)和
`cloud_sync_web.dart`(web 走 yjs)是同一套协议的两个实现——"一个权威引擎全平台跑"
靠的是这两份保持一致,只改一边等于埋雷。
