# Mica 开发环境与辅助工具

> 换机/新机重配环境的清单。核心工具链(Flutter、Rust、Docker、平台 SDK)见 `docs/desktop-plan.md` 的「环境备忘」;发版/构建工具链(just、Inno Setup、docker login)见 `docs/release.md`;本文专记 **AI 辅助开发工具**(MCP、codebase-memory-mcp、skills)。

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

**不自动更新** —— 大改后手动重建:`index_repository(repo_path="D:/codes/mica", mode="full")`(模式 `full` / `moderate` / `fast`);`index_status` 查新鲜度,`list_projects` 列项目(本项目名 `D-codes-mica`)。

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

## GitHub MCP(用户级,OAuth)

```bash
claude mcp add --transport http --scope user github https://api.githubcopilot.com/mcp/
```

加完在 Claude Code 里跑 `/mcp` 选 github 完成 GitHub OAuth 授权(令牌不入配置文件);
授权前 `claude mcp list` 会显示未连接。本机 `gh` CLI 已登录(`weironz`),也可改用
带 `gh auth token` 的本地 server 方案。

## Windows 桌面构建前置

- **Flutter SDK**(本机 `C:\flutter`,已入 PATH)、**VS Build Tools + Desktop C++ workload + Win10 SDK**(`flutter build windows` 需要)。
- **开发者模式(Developer Mode)**:一旦项目含**任何原生插件**(如 `window_manager`),Flutter 在 Windows 上用符号链接管理插件,需开开发者模式,否则 `pub get`/构建报 `Building with plugins requires symlink support`。开法:`start ms-settings:developers` → 打开「开发人员模式」(普通用户即可,无需重启)。M1 无插件时不需要,M2 起需要。
- **Docker Desktop**:跑本地后端(端口原生发布到 Windows localhost)。

## 重配顺序(换机)

1. 核心工具链:Flutter、Rust、Docker Desktop、平台 SDK + **开发者模式**(见上「Windows 桌面构建前置」)。
2. 本文的 MCP(项目级 `.mcp.json` = playwright;用户级 codebase-memory-mcp 等)、首次 `index_repository` 建图、skills。
3. 起后端:`docker compose up -d --build postgres api`(详见 `docs/desktop-plan.md` M1 状态)。
