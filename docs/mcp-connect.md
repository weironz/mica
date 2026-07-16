# 把 Mica 接入 Claude Code / Claude Desktop(MCP)

`mica-cli mcp` 把 Mica 的 REST API 以 MCP 工具的形式暴露给任何 MCP 客户端:
列出/搜索/读取页面、新建文档、往已有文档追加或改写内容、移动、软删、导出。
一个典型用法:**让 Claude Code 把对话内容直接写进 Mica 文档**。

它是薄代理 —— 用 PAT 调 REST API,自身不碰数据库或存储;和 `mica-cli` 的
其他子命令共用一个二进制(CI 每平台只发一个 artifact),也共用同一条凭证
解析链。实现在 `crates/mcp-server`(库)+ `crates/cli` 的 `mcp` 子命令。

## 一次性准备

先装 `mica-cli`(安装/更新方式以 [`cli.md`](cli.md#install) 为准 —— Windows 一键:
`irm https://raw.githubusercontent.com/weironz/mica/main/install.ps1 | iex`)。然后:

```bash
# 登录并创建一个长期 token(写权限;--expires-days 可选)
mica-cli auth login --server https://mica.cloudcele.com --email you@example.com
mica-cli auth token create --name claude-code --scope read --scope write
# 记下输出里的 "token": "mica_pat_…" —— 只显示这一次
```

## 最快:让 mica-cli 自己配

装好 `mica-cli`(见 [`cli.md`](cli.md#install))后,一条命令写好客户端配置 + 生成并嵌入
token,免去手改 JSON/TOML:

```bash
mica-cli auth login --server https://mica.cloudcele.com --email you@example.com
mica-cli mcp install --client claude-desktop   # 或 claude-code / cursor / codex / gemini / windsurf
mica-cli mcp install --all                     # 本机装了的客户端一次全配
```

它把 `mica` 这个 MCP server **合并**进目标客户端配置(保留其它条目)、指向当前 mica-cli
二进制、默认建一个 PAT 写进去。配好重启客户端即可。细节(`--no-token`/`--pat`/`--dry-run`、
各客户端配置路径)见 [`cli.md`](cli.md#mcp--model-context-protocol-server-for-ai-clients)。

下面是**手动**配法(想自己控制,或客户端尚未被 `mcp install` 支持时)。

## Claude Code(手动)

```bash
claude mcp add mica --scope user \
  -e MICA_API_BASE_URL=https://mica.cloudcele.com \
  -e MICA_PAT=mica_pat_… -- /path/to/mica-cli mcp
```

> **`--scope user` 不能省 —— 尤其在 Windows 上。** 省掉它就是默认的 `local`
> (项目级),而 Windows 上 `claude mcp add` 把 project key 写成**正斜杠**
> (`C:/Users/you/proj`),应用自己建的 project key 却是**反斜杠**
> (`C:\Users\you\proj`)。两个 key 对不上,配置就永远加载不到。
>
> 实测(2026-07,任意 shell 都复现):CLI 打印 `[project: C:\Users\willz\probe]`,
> 写进 `~/.claude.json` 的却是 `'C:/Users/willz/probe'`。
>
> **症状极具迷惑性**:`claude mcp list` 报 `mica: ✓ Connected`(它读的是自己写的
> 那个正斜杠 key,当然找得到、也真能把 server 拉起来),但**重启后会话里一个
> mica 工具都没有** —— 两边都没说谎,只是在看不同的配置。别信 `✓ Connected`,
> 以「重启后会话里有没有 `mica_*` 工具」为准。
>
> `--scope user` 写的是 `~/.claude.json` **顶层**的 `mcpServers`,不挂 project
> key,从根上绕开这个坑;代价是所有项目都能看到 mica(对笔记工具而言通常正是
> 你要的)。**PAT 也别写进项目里的 `.mcp.json`** —— 那是会被 git 跟踪的文件,
> 仓库一旦公开,token 就跟着提交上去了。

或手写进 `~/.claude.json`(顶层 `mcpServers`)/ 项目 `.mcp.json`(**不要放
token**):

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
| `mica_search` | 按**标题 + 正文**搜页面,返回命中片段(`title_match` 说明是哪边命中) |
| `mica_read_document` | 读文档(Markdown) |
| `mica_get_outline` | 大纲(标题+block id,`insert_at` 的锚点来源) |
| `mica_create_document` | 建页面(可带 Markdown 正文) |
| `mica_update_document` | 写入已有文档:`append`(默认、安全)/ `replace_all` / `insert_at` / `find_replace` |
| `mica_add_image` | 从 http(s) URL 把图片**存进**工作区,返回 file_id + 可直接粘的 Markdown |
| `mica_read_image` | 按 file_id 取回图片**像素**(MCP ImageContent),让模型真的看见 |
| `mica_rename` | 重命名页面或文件夹(只改名,不动内容) |
| `mica_create_folder` | 建文件夹(纯容器),返回 view_id 好往里放东西 |
| `mica_move_document` | 移动页面(改父) |
| `mica_reorder` | 一次调用重排某文件夹(或顶层)的所有子项 —— 传有序 view_id 列表;排序/整理用它 |
| `mica_trash_view` | 软删到回收站(需 `confirm: true`) |
| `mica_list_trash` | 列回收站(找要恢复的东西) |
| `mica_restore_view` | 从回收站恢复页面/文件夹(撤销软删) |
| `mica_list_versions` | 列文档的命名版本(可回滚的检查点),不含原始编辑日志 |
| `mica_create_version` | 把文档当前状态钉成命名版本(改前打检查点) |
| `mica_restore_version` | 回滚文档到某版本(仍 append-only,可再撤销) |
| `mica_export_workspace` | 导出整个工作区为 Markdown |

## 用例:把对话存进 Mica

对 Claude 说"把这段对话记到 mica 里",它会:

1. `mica_list_workspaces` → 选工作区
2. `mica_create_document(name: "2026-07-16 与 Claude 的对话", markdown: 首轮内容)`
3. 之后每轮 `mica_update_document(mode: "append", markdown: 新内容)`

Markdown 里的 `$…$` 行内公式、`$$…$$` 块级公式、代码块、表格都会被服务端
解析成对应的块/标记,在客户端里正常排版。

### 图片:`![](https://…)` 只是热链,不是存进来

直接写 `![](https://example.com/x.png)`,Mica **只存这个 URL**,字节从没进来过 ——
源站挂了图就没了,离线也看不到。要真正存进来用 `mica_add_image`:

1. `mica_add_image(workspace_id, url: "https://…")`
   → `{"ok":true,"file_id":"…","markdown":"![name](/api/workspaces/…/files/…/blob/name)"}`
2. 把返回的 `markdown` 原样写进文档。

相同字节按 sha256 去重,不会存两份。

**读**:`mica_read_document` 里的图片就是这种 blob 路径
`![alt](/api/workspaces/{ws}/files/{file_id}/blob/{name})`。把里面的 `file_id` 交给
`mica_read_image` 就能拿到真实像素(MCP ImageContent);路径本身也能直接在浏览器里
打开。上限 4MB —— MCP 的图片是一整条 base64,没有流式。

**往返是闭合的**:导出写出 blob 路径,导入认得自己的路径并还原成 file_id 引用
(不是外链)。只认**本工作区**的路径 —— 别的工作区的 blob 不归这里引用,否则那边的
GC 会把字节回收掉,这边的页面就烂了。

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
- **`claude mcp list` 说 `✓ Connected`,但会话里没有 `mica_*` 工具**:配置写进了
  应用读不到的 project key(Windows 正/反斜杠不一致,见上面 Claude Code 一节)。
  用 `--scope user` 重新加一遍:
  ```bash
  claude mcp remove mica --scope local
  claude mcp add mica --scope user -e MICA_API_BASE_URL=… -e MICA_PAT=… -- /path/to/mica-cli mcp
  ```
  验证不要看 `mcp list`,直接查 `~/.claude.json` 的**顶层** `mcpServers` 里有没有
  `mica`,然后重启 Claude Code。
- `contains a raw control character where a LaTeX command should be`:公式的
  反斜杠在 JSON 里没双写(`"$\times$"` 被解析成 TAB+`imes`)。什么都没写入,
  双写成 `"$\\times$"` 重发即可。见上面「公式里的反斜杠必须在 JSON 里双写」。
