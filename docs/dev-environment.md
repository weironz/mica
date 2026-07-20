# Mica 开发环境与辅助工具

> **新机器从零搭建请先看 [`bootstrap.md`](bootstrap.md)** —— 那份是按顺序照做的完整清单(系统前置、
> 工具链、国内镜像、MCP、验收、实测耗时)。**本文是参考层**:同样的东西讲得更细,并记录每条规则的来历
> 和踩过的坑。两份有出入时以 `bootstrap.md` 的命令为准(它是实测验证过的)。
>
> 另见:发版/构建工具链(Inno Setup、docker login)→ `docs/release.md`;桌面端里程碑 → `docs/desktop-plan.md`。

## MCP 服务器

分两层:**项目级**随仓库走(`.mcp.json`,提交进库);**用户级**是机器全局,换机要各自重配。

### 项目级(`.mcp.json`,已入库)

| 服务 | 命令 | 备注 |
|---|---|---|
| playwright | `npx -y @playwright/mcp@latest --headless` | web 端实测截图 |

### 用户级(全局配置,换机单独重配)

这些**不在仓库**里,配在 Claude Code 用户级(`claude mcp add ...` 或用户 settings):

- **codebase-memory-mcp** — 代码知识图谱(见下节;CLAUDE.md 要求探索代码优先用它)
- **GitHub MCP** — 仓库/PR/issue 操作
- **Playwright MCP** — 浏览器自动化(与项目级重复,按需)
- **Chrome DevTools MCP** — 调试/性能/网络
- **Context7 MCP** — 实时拉取库/框架文档(全局规则要求查库文档时优先用它)

> 迁移时用 `claude mcp list` 导出现有配置作为权威来源,本表仅作清单提醒。

## codebase-memory-mcp(知识图谱)

CLAUDE.md 要求探索代码**优先用图谱**而非 Grep/Read(省 token + 给结构上下文:callers/callees/数据流/测试)。**用户级 MCP**,全局装的 exe(`AppData\Local\Programs\codebase-memory-mcp\`;`claude mcp get codebase-memory-mcp` 看权威配置),不在 `.mcp.json` 里。

工具面:`search_graph`(找符号)/ `search_code`(图增强搜)/ `get_code_snippet`(取源码)/ `trace_path`(调用链/数据流)/ `query_graph`(Cypher)/ `get_architecture` / `detect_changes`(审改动)。

**不自动更新** —— 大改后手动重建:`index_repository(repo_path="C:/data/codes/mica-will-laptop", mode="full")`(模式 `full` / `moderate` / `fast`);`index_status` 查新鲜度,`list_projects` 列项目(本项目名 `C-data-codes-mica-will-laptop`,由仓库路径推导)。**查询类工具都要带 `project` 参数**,不带会直接报 `missing required argument: project`。

> 项目名跟着仓库路径走:上一台主力机是 `D:/codes/mica` / `D-codes-mica`,那台机器硬件故障报废后于 2026-07-20 迁到本机,旧名已失效。
>
> ⚠️ `full` 模式偶发 worker 崩溃(返回 `{"status":"error","outcome":"exit_nonzero"}`,而 `logs/.worker-*.log` 是**空文件**,查不出是哪个文件触发的)。原样重试一次即可,不用降级到 `fast`——实测重试后 `full` 正常跑完(8979 节点 / 37710 边,`fast` 只有 5773 / 27559)。

配套用户级 hook(`~/.claude/hooks/`):`cbm-code-discovery-gate`(PreToolUse Grep|Glob 门,督促走图)+ `cbm-session-reminder`(SessionStart 注入探索协议 + 近期上下文)。

> code-review-graph 已于 2026-07-15 全局卸载(省 token),由本 MCP 取代;`.mcp.json` / `.claude` hooks / pipx 本体 / `.code-review-graph` 数据均已移除。

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

## Windows 桌面构建前置

- **Flutter SDK**(本机 `C:\flutter`,已入 PATH)、**VS Build Tools + Desktop C++ workload + Win10 SDK**(`flutter build windows` 需要)。
- **开发者模式(Developer Mode)**:一旦项目含**任何原生插件**(如 `window_manager`),Flutter 在 Windows 上用符号链接管理插件,需开开发者模式,否则 `pub get`/构建报 `Building with plugins requires symlink support`。开法:`start ms-settings:developers` → 打开「开发人员模式」(普通用户即可,无需重启)。M1 无插件时不需要,M2 起需要。

  ⚠️ **别用 PowerShell 的 `New-Item -ItemType SymbolicLink` 验证是否生效**:Windows PowerShell 5.1 建符号链接时不传 `SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE`,即使开发者模式已开也会报 `Administrator privilege required`,是**假阴性**。用 `cmd /c mklink /D <链接> <目标>` 验证才准(Flutter 走的也是带该标志的 Win32 API)。真正的开关状态查注册表:`HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock` 的 `AllowDevelopmentWithoutDevLicense` == 1。
- **Docker Desktop**:跑本地后端(端口原生发布到 Windows localhost)。

## 重配顺序(换机)

1. 核心工具链:Flutter、Rust、Docker Desktop、平台 SDK + **开发者模式**(见上「Windows 桌面构建前置」)。
2. 本文的 MCP(项目级 `.mcp.json` = playwright;用户级 codebase-memory-mcp 等)、首次 `index_repository` 建图、skills。
3. 起后端:`docker compose up -d --build postgres api`(详见 `docs/desktop-plan.md` M1 状态)。

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
