# 把 Mica 接入 Claude Code / Claude Desktop(MCP)

`mica-cli mcp` 把 Mica 的 REST API 以 MCP 工具的形式暴露给任何 MCP 客户端:
列出/搜索/读取页面、新建文档、往已有文档追加或改写内容、移动、软删、导出。
一个典型用法:**让 Claude Code 把对话内容直接写进 Mica 文档**。

它是薄代理 —— 用 PAT 调 REST API,自身不碰数据库或存储;和 `mica-cli` 的
其他子命令共用一个二进制(CI 每平台只发一个 artifact),也共用同一条凭证
解析链。实现在 `crates/mcp-server`(库)+ `crates/cli` 的 `mcp` 子命令。

## 一次性准备

```bash
# 1. 拿二进制:GitHub Release 下载 mica-cli-<版本>-<平台>,或本地构建
cargo build -p mica-cli --release

# 2. 登录并创建一个长期 token(写权限;--expires-days 可选)
mica-cli auth login --server https://mica.cloudcele.com --email you@example.com
mica-cli auth token create --name claude-code --scope read --scope write
# 记下输出里的 "token": "mica_pat_…" —— 只显示这一次
```

## Claude Code

```bash
claude mcp add mica -e MICA_API_BASE_URL=https://mica.cloudcele.com \
  -e MICA_PAT=mica_pat_… -- /path/to/mica-cli mcp
```

或手写进 `~/.claude.json` / 项目 `.mcp.json`:

```json
{
  "mcpServers": {
    "mica": {
      "command": "/path/to/mica-cli",
      "args": ["mcp"],
      "env": {
        "MICA_API_BASE_URL": "https://mica.cloudcele.com",
        "MICA_PAT": "mica_pat_…"
      }
    }
  }
}
```

Claude Desktop 的 `claude_desktop_config.json` 用同一段 `mcpServers`。

**凭证解析链**(先到先得):`MICA_API_BASE_URL` / `MICA_PAT` →
`--server` / `MICA_SERVER` / `MICA_TOKEN` → `mica-cli auth login` 存下的配置。
所以在已登录的机器上,`"command": "mica-cli", "args": ["mcp"]` 且**不带 env**
也能直接用(walks the same chain as every other subcommand)。

只读接入(例如给一个只查资料的 agent):`args: ["mcp", "--read-only"]`,
或 `MICA_MCP_READ_ONLY=1` —— 写工具在调用时拒绝,读工具照常。

## 工具一览

| 工具 | 作用 |
| --- | --- |
| `mica_list_workspaces` | 列工作区(id/name/role) |
| `mica_list_pages` | 页树(文档+文件夹;拿 object_id / view id) |
| `mica_search` | 按标题搜页面 |
| `mica_read_document` | 读文档(Markdown) |
| `mica_get_outline` | 大纲(标题+block id,`insert_at` 的锚点来源) |
| `mica_create_document` | 建页面(可带 Markdown 正文) |
| `mica_update_document` | 写入已有文档:`append`(默认、安全)/ `replace_all` / `insert_at` / `find_replace` |
| `mica_move_document` | 移动页面 |
| `mica_trash_view` | 软删到回收站(需 `confirm: true`) |
| `mica_export_workspace` | 导出整个工作区为 Markdown |

## 用例:把对话存进 Mica

对 Claude 说"把这段对话记到 mica 里",它会:

1. `mica_list_workspaces` → 选工作区
2. `mica_create_document(name: "2026-07-16 与 Claude 的对话", markdown: 首轮内容)`
3. 之后每轮 `mica_update_document(mode: "append", markdown: 新内容)`

Markdown 里的 `$…$` 行内公式、`$$…$$` 块级公式、代码块、表格都会被服务端
解析成对应的块/标记,在客户端里正常排版。

### ⚠️ 公式里的反斜杠必须在 JSON 里双写

工具参数走 JSON。JSON 自己就用反斜杠转义,于是 LaTeX 命令和 JSON 转义**撞车**:

| JSON 里写 | 实际解析成 | 结果 |
|---|---|---|
| `"$\times$"` | TAB + `imes` | ❌ 公式毁了 |
| `"$\frac{a}{b}$"` | 换页符 + `rac{a}{b}` | ❌ |
| `"$\\times$"` | `\times` | ✅ |

**这不会报 JSON 语法错** —— `\t`/`\f` 都是合法转义,所以坏数据会被安静地存下来。
`\eta` 反而能活(`\e` 不是合法转义),于是常见症状是**一条公式里有的命令没了、有的
还在**,渲染成红色报错。踩雷的命令很多:`\times \theta \tau \frac \forall \beta
\vec \nu \nabla \rho …`。

写工具(`mica_create_document` / `mica_update_document`)现在会**拦下**这种输入并
报错说明,不会写进去 —— 收到该错误就把反斜杠双写后重发。`\n`/`\r` 拦不住(和真
换行无法区分),但它们会直接把公式撑断,肉眼可见。

## 排错

- `no token`:三条链都没解析到凭证 —— 传 `MICA_PAT`,或先 `auth login`。
- `Mica API 401`:PAT 被吊销或过期,`mica-cli auth token list` 查,重建一个。
- 写工具报 read-only:去掉 `--read-only` / `MICA_MCP_READ_ONLY`,且 PAT 要有
  `write` scope(`token create --scope read --scope write`)。
- 协议日志:stderr(stdout 是 JSON-RPC 通道,永远干净);`RUST_LOG=debug`
  提高日志级别。
- `contains a raw control character where a LaTeX command should be`:公式的
  反斜杠在 JSON 里没双写(`"$\times$"` 被解析成 TAB+`imes`)。什么都没写入,
  双写成 `"$\\times$"` 重发即可。见上面「公式里的反斜杠必须在 JSON 里双写」。
