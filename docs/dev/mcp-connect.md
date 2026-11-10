# 把 Mica 接入 AI Agent(MCP)

`mica-cli mcp` 把 Mica 以 [MCP](https://modelcontextprotocol.io) 工具的形式提供给任何支持 MCP 的 AI 客户端:
搜索和读取页面、新建文档、局部改写、整理页树、版本与回收站。典型用法是**让 Agent 把内容直接写进 Mica 文档**。

- **官方服务地址**:`https://mica.cloudcele.com`(需要已有账号;自建服务器换成你自己的地址)。
- **传输方式**:本机 stdio —— 客户端启动 `mica-cli mcp` 这个子进程。没有远程 HTTP / SSE 端点。
- **凭证**:个人访问令牌(PAT,`mica_pat_…`)。MCP 进程只是用它调 REST API,自己不碰数据库。

设计取舍见 [mcp-server.md](mcp-server.md);不能运行本地程序的 Agent 请直接用 [HTTP API](api.md)。

## 1. 三步接入

**① 安装 `mica-cli`**(详见 [cli.md](cli.md)):

```bash
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/weironz/mica/main/install.sh | sh
# Windows PowerShell
irm https://raw.githubusercontent.com/weironz/mica/main/install.ps1 | iex
```

**② 登录并创建令牌**:

```bash
mica-cli auth login --server https://mica.cloudcele.com --email you@example.com
mica-cli auth token create --name my-agent --scope read --scope write
# 输出里的 mica_pat_… 只显示这一次,记下来
```

只读 Agent 只给 `--scope read`。

**③ 在客户端里登记**。所有客户端要的都是同一件事 —— 一条启动命令加两个环境变量:

| 项 | 值 |
| --- | --- |
| 命令 | `mica-cli` 的**绝对路径**(`which mica-cli` / `where mica-cli`) |
| 参数 | `mcp`(只读接入用 `mcp --read-only`) |
| `MICA_API_BASE_URL` | `https://mica.cloudcele.com` |
| `MICA_PAT` | `mica_pat_…` |

在已经 `auth login` 过的机器上,两个环境变量都可以省掉 —— 配置文件里就不存令牌,这是最推荐的写法。

接好后让 Agent 调一次 `mica_list_workspaces`,能列出工作区就是通了。

## 2. 各客户端的写法

配置文件的位置以各客户端自己的文档为准(它们会变);下面只给格式。

**Claude Code**

```bash
claude mcp add --scope user mica -- /path/to/mica-cli mcp
# 要把地址和令牌写进配置时:
claude mcp add --scope user mica \
  -e MICA_API_BASE_URL=https://mica.cloudcele.com -e MICA_PAT=mica_pat_… \
  -- /path/to/mica-cli mcp
```

`--scope user` 不要省:Windows 上默认的项目级配置会因为路径斜杠方向不一致而加载不到,
表现为 `claude mcp list` 显示已连接、会话里却没有 `mica_*` 工具。

**Claude Desktop / Cursor / Windsurf / Cline / Gemini CLI**(都是 `mcpServers` 这一段 JSON)

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

**VS Code(GitHub Copilot)**:同样的内容,顶层键是 `servers`,并加 `"type": "stdio"`。

**Codex CLI**(`~/.codex/config.toml`)

```toml
[mcp_servers.mica]
command = "/path/to/mica-cli"
args = ["mcp"]
env = { MICA_API_BASE_URL = "https://mica.cloudcele.com", MICA_PAT = "mica_pat_…" }
```

**自己写的 Agent(任意 MCP SDK)**:用 SDK 的 stdio 客户端启动上面那条命令即可。以 Python 为例:

```python
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

params = StdioServerParameters(
    command="/path/to/mica-cli", args=["mcp"],
    env={"MICA_API_BASE_URL": "https://mica.cloudcele.com", "MICA_PAT": "mica_pat_…"},
)
async with stdio_client(params) as (read, write), ClientSession(read, write) as session:
    await session.initialize()
    print(await session.call_tool("mica_list_workspaces", {}))
```

> **令牌安全**:不要把 PAT 写进会被 git 跟踪的文件(如项目里的 `.mcp.json`)。写在命令行里的令牌会留在 shell 历史。
> 泄露了就 `mica-cli auth token list` → `auth token revoke <id>` → 重建。

## 3. 给 Agent 的使用要点

连接时服务端会把一份使用说明(`instructions`)发给客户端,每个工具也带自己的描述 —— 那是权威。这里是最要紧的几条:

- **内容是 Markdown**。读出来是 Markdown,写进去也是 Markdown;公式(`$…$`、`$$…$$`)、代码块、表格都会被解析成对应的块。
- **找东西先搜索**:`mica_list_workspaces` → `mica_search`(标题、文件夹名、正文)→ `mica_read_document` / `mica_get_outline`。
  `mica_list_pages` 返回整棵页树,大工作区里最贵,只在确实需要整体结构时用。
- **两种 id**:读写文档用 `document_id`;移动、改名、删除用 `view_id`。两者都在搜索和页树结果里。
- **改已有文档优先局部改**:`mica_update_document` 的 `append`(默认,最安全)、`insert_at`、`find_replace`、`delete`;
  多处修改用 `mica_apply_edits` 一次提交。`replace_all` 会覆盖整篇,只在确实要重写时用。
- **防并发覆盖**:`mica_get_outline` 返回 `seq`,写入时带上 `expected_seq`;期间文档被别人改过会得到 409,重新读再决定。
- **危险操作都要 `confirm: true`**。进回收站可恢复;`purge` 类、清空回收站、删工作区、删评论、删文件不可恢复。
- **批量编辑不是事务**:`mica_apply_edits` 部分失败时返回已生效的项和 `failed_index`,重试时不要重复已生效的部分。

### 两个容易踩的坑

**公式里的反斜杠要在 JSON 里双写。** 工具参数是 JSON,`"$\times$"` 会被解析成制表符加 `imes`,而且不报语法错。
写成 `"$\\times$"`。写工具会拦下这种输入并报错(什么都不写入),按提示双写后重发即可。

**`![](https://…)` 只是外链,图片没有存进来。** 要存进 Mica:先 `mica_add_image(workspace_id, url)`,
把返回的 `markdown` 原样写进文档。读图用 `mica_read_image(file_id)`(上限 4 MB)。

### `find_replace` 的边界

- 在**单个块或单个表格单元格**内匹配可见文本,不跨块、不跨单元格;加粗、行内代码等格式不会截断匹配。
- `replace` 默认按行内 Markdown 解析;要原样插入传 `replacement_format: "plain_text"`。
- `unique: true` 要求恰好命中一处,否则整次拒绝。
- 换段、插表格这类结构变化用 `insert_at` / `delete`。

## 4. 工具一览(39 个)

| 类别 | 工具 |
| --- | --- |
| 读 | `mica_list_workspaces`、`mica_list_pages`、`mica_search`、`mica_read_document`、`mica_read_documents`、`mica_get_outline` |
| 写 | `mica_create_document`、`mica_create_workspace`、`mica_update_document`、`mica_apply_edits`、`mica_add_image`、`mica_read_image` |
| 整理 | `mica_rename`、`mica_create_folder`、`mica_move_document`、`mica_move_views`、`mica_reorder`、`mica_clone_document`、`mica_transfer_view` |
| 版本 | `mica_list_versions`、`mica_create_version`、`mica_restore_version` |
| 回收站 | `mica_trash_view`、`mica_trash_views`、`mica_list_trash`、`mica_restore_view`、`mica_restore_views`、`mica_purge_view`、`mica_purge_views`、`mica_empty_trash` |
| 分享与导出 | `mica_share`、`mica_unshare`、`mica_export_workspace` |
| 协作与管理 | `mica_list_comments`、`mica_delete_comment`、`mica_list_members`、`mica_remove_member`、`mica_delete_file`、`mica_delete_workspace` |

每个工具的参数和说明以客户端里看到的 schema 为准;删除相关的边界见 [mcp-server.md](mcp-server.md)。

## 5. 凭证怎么解析

先到先得,对 `mica-cli` 的所有子命令都一样:

- **服务器**:`MICA_API_BASE_URL` → `--server` / `MICA_SERVER` → `auth login` 存下的配置
- **令牌**:`--token` → `MICA_PAT` → `auth login` 存下的配置

空值等同未设置;首尾空白会被去掉。环境变量总能盖过本机已登录的账号。

## 6. 排错

| 现象 | 原因与处理 |
| --- | --- |
| `not signed in` / `no server` | 没解析到凭证或地址。设 `MICA_PAT` 和 `MICA_API_BASE_URL`,或先 `auth login`。先确认变量不是空串。报错里点名 `MICA_TOKEN` 说明用了已停用的变量名,改成 `MICA_PAT` |
| 已登录,但 MCP 里仍然 `no server` | 客户端过滤了子进程的环境变量(如 `HOME`、`APPDATA`),找不到登录配置。在 `env` 里把 `MICA_CONFIG` 设为 `config.json` 的绝对路径,或直接给两个环境变量 |
| `Mica API 401` | 令牌被吊销或过期。`mica-cli auth token list` 查看,重建一个 |
| 写工具报 read-only | 去掉 `--read-only` / `MICA_MCP_READ_ONLY`,并确认令牌有 `write` 权限 |
| Claude Code 显示已连接但没有 `mica_*` 工具 | 没用 `--scope user`。`claude mcp remove mica --scope local` 后按第 2 节重新添加,再重启 |
| `contains a raw control character where a LaTeX command should be` | 公式的反斜杠没双写,见第 3 节 |
| `find_replace` 被拒绝并提示 API 版本 | 服务器比 CLI 旧。升级服务器,或换用 `insert_at` / `delete` |
| 升级了 CLI 但行为没变 | 客户端还在用旧进程,重连或重启客户端 |

协议日志在 stderr(stdout 是 JSON-RPC 通道);`RUST_LOG=debug` 可提高日志级别。

本地回归测试:`scripts/test-mcp-edits.py --cli <mica-cli> --api <mica-api-server>`
(需要一个库名以 `mica_mcp_` 开头的专用临时 Postgres,通过 `DATABASE_URL` 指定)。
