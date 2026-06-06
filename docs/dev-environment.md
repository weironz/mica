# Mica 开发环境与辅助工具

> 换机/新机重配环境的清单。核心工具链(Flutter、Rust、Docker、平台 SDK)见 `docs/desktop-plan.md` 的「环境备忘」；本文专记 **AI 辅助开发工具**(MCP、code-review-graph、skills)。

## MCP 服务器

分两层:**项目级**随仓库走(`.mcp.json`,提交进库);**用户级**是机器全局,换机要各自重配。

### 项目级(`.mcp.json`,已入库)

| 服务 | 命令 | 备注 |
|---|---|---|
| code-review-graph | `uvx code-review-graph serve` | 知识图谱(见下节);`.mcp.json` 里 `cwd` 写的是 Linux 路径 `/data/codes/mica`,**换机需改成本机仓库路径**(如 Windows `D:\codes\mica`) |
| playwright | `npx -y @playwright/mcp@latest --headless` | web 端实测截图 |

### 用户级(全局配置,换机单独重配)

这些**不在仓库**里,配在 Claude Code 用户级(`claude mcp add ...` 或用户 settings):

- **GitHub MCP** — 仓库/PR/issue 操作
- **Playwright MCP** — 浏览器自动化(与项目级重复,按需)
- **Chrome DevTools MCP** — 调试/性能/网络
- **Context7 MCP** — 实时拉取库/框架文档(全局规则要求查库文档时优先用它)

> 迁移时用 `claude mcp list` 导出现有配置作为权威来源,本表仅作清单提醒。

## code-review-graph(知识图谱)

CLAUDE.md 要求探索代码**优先用图谱**而非 Grep/Read。安装与建图:

```bash
pipx install code-review-graph
code-review-graph install          # 装 hooks(图谱随文件改动自动增量更新)
code-review-graph build            # 首次全量建图(在仓库根目录跑)
```

MCP 服务端本身由 `.mcp.json` 的 `uvx code-review-graph serve` 拉起。

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

## 重配顺序(换机)

1. 核心工具链:Flutter、Rust、Docker Desktop、平台 SDK(见 `docs/desktop-plan.md`)。
2. 本文的 MCP(改 `.mcp.json` 的 cwd)、code-review-graph 建图、skills。
3. 起后端:`docker compose up -d --build postgres api`(详见 `docs/desktop-plan.md` M1 状态)。
