<h1 align="center">Mica</h1>

<p align="center">
  <em>你的笔记就是 Markdown 文件。存在你的硬盘,或者你自己的服务器上。</em>
</p>

<p align="center">
  <a href="https://github.com/weironz/mica/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/weironz/mica?label=download&color=2f7d6f"></a>
  <a href="https://github.com/weironz/mica/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/weironz/mica/actions/workflows/ci.yml/badge.svg"></a>
  <a href="#-许可证"><img alt="License" src="https://img.shields.io/badge/license-AGPL--3.0-2f7d6f"></a>
  <img alt="Platform" src="https://img.shields.io/badge/platform-Windows%20%7C%20Web-2f7d6f">
</p>

<p align="center">
  <a href="#-安装">下载</a> ·
  <a href="#-自托管">自托管</a> ·
  <a href="docs/">文档</a> ·
  <a href="docs/roadmap.md">路线图</a>
</p>

<p align="center">
  <a href="README.md">English</a> | <b>简体中文</b>
</p>

<!-- SCREENSHOT: 编辑器宽幅截图放这里,放在所有内容之前。 -->

---

Mica 是一个 Markdown 工作区。笔记以 Markdown 存储——存在你自己硬盘上的一个文件夹里,
或者存在你自己跑的 Mica 服务端上——整个应用是围绕"让这个格式双向保真"来构建的。

## ✨ 特性

**两个世界,一个应用。** *本地*工作区就是你硬盘上的一个 Markdown 文件夹:不需要账号、
不联网、不同步。*云端*工作区跑在 Mica 服务端上,实时同步,可以分享。两者显式切换,
且一次只显示一个,所以一个页面存在哪里始终是明确的。

**编辑器是画出来的,不是拼出来的。** 它是单个 Flutter `RenderBox`,自己绘制文字、光标、
选区和块,底层是 marks-over-plain-text 模型——不是控件树,不是 `WebView`,
也不是包了一层的第三方编辑器。因此光标和选区在所有平台上行为一致,中日韩 IME 组合输入
原生可用,长文档也能保持流畅(绘制按视口裁剪)。

**全平台同一个 CRDT 引擎。** 同步基于 `yrs`(Yjs 的 Rust 移植)。Web 端直接用 Yjs 本体,
两者在 update、state-vector 和 lib0 编码层**字节兼容**——所以每个平台讲的是同一份权威格式,
而不是它的某种转译。

**Rust 数据面。** 一切解析文件、遍历压缩包、做哈希、和存储打交道的活都在 Rust 里跑。
Dart 负责绘制、光标与选区、命中测试,以及编辑器的延迟敏感路径。

### 编辑

- **块** —— 标题、列表、表格、引用、任务列表、脚注。
- **代码** —— 围栏代码块,带语法高亮。
- **公式** —— LaTeX,行内和块级。行内公式按基线排版,随字号缩放。
- **图表** —— Mermaid,由纯 Rust 引擎离线渲染(不需要浏览器,不需要 Node)。
- **图片** —— 粘贴或拖入即上传,按 SHA-256 内容寻址并去重。
- **键盘优先** —— 边敲边生效的输入规则、粘贴转块、复制为 Markdown。

### 工作区

- **文件夹与页面** —— 拖拽排序和移动;工作区顺序按用户各自保存。
- **实时协作** —— 协作者在线状态、实时光标。
- **离线可用** —— 编辑排队、重连后收敛;离线插入的图片等网络恢复再上传。
- **版本历史** —— 自动快照 + 命名检查点,带只读预览和块级 diff。
- **公开链接** —— 只读分享单个文档。
- **全文搜索** —— 跨整个工作区。

### 数据进出

- **导入** —— Markdown 文件、文件夹、ZIP,以及 **Notion 导出包**(自动去掉 ID 后缀、
  移除重复的 H1 标题、展开嵌套的 `Part-N.zip`)。
- **导出** —— Markdown、HTML、PDF,以及单页 / 文件夹 / 单个工作区 / 全部工作区的 ZIP。
- **无损往返** —— 导出再导入,树结构、名称、资源和链接原样还原。
- **MCP 服务** —— 让 Claude 或任意 MCP 客户端读取、搜索、创建和编辑页面。
- **可选 AI** —— `/` 命令和全局输入框,走 Anthropic Messages API。没配 key 时整个功能隐藏。

### Markdown 保真度

底座是 CommonMark 0.31.2 —— **读侧 641/641 规范用例全过** —— 加 GFM(**24/24**),
再加一小层方言补 GFM 表达不了的东西:脚注、front matter、Pandoc 数学约定。
写侧输出规范化子集,**round-trip 是被 CI 地板钉住的不变量**。
见[记分牌](docs/commonmark-scoreboard.md)。

遇到 GFM 没有对应表示的特性,规则是:序列化成**合法的 GFM**,保证在任何第三方阅读器里
都能正常渲染——绝不发明别人会渲染错的语法——同时把无损形式带在带外,
这样重新导入我们自己的导出时,能把 GFM 丢掉的部分还原回来。

## 📦 安装

**桌面版** —— 从 [Releases](https://github.com/weironz/mica/releases/latest)
下载 `Mica-Setup-*.exe`。应用会从同一来源自动更新。

**命令行** —— 每个 release 都附带 Windows / Linux / macOS 三个平台的 `mica-cli`。
它走同一套 API,并承载 MCP 服务(`mica-cli mcp`)。见 [docs/cli.md](docs/cli.md)。

**Web** —— 自托管,见下。

## 🏗 自托管

`deploy/` 下有两份 compose。单机版是**自带全套的**——PostgreSQL 和 RustFS
(S3 兼容存储)跟应用一起跑,不需要你自备。

<details>
<summary><b>单机部署 —— nginx 占 80 端口,不需要 Traefik</b></summary>

```sh
cp deploy/.env.prod.example .env.prod
vi .env.prod          # 填 SERVER_IP,以及足够强的 JWT_SECRET 和各种口令
                      #   openssl rand -hex 32
./deploy/deploy.sh    # 构建 Flutter bundle + API 镜像,然后全部启动
```

然后打开 `http://<SERVER_IP>/`。对外端口是 **80**(应用)和 **9000**(RustFS——
浏览器直接对它做预签名上传下载,所以 `S3_ENDPOINT` **必须是浏览器可达的地址**)。
Postgres 只在 compose 网络内部可达。

`./deploy/deploy.sh --web-only` 只重建 Flutter bundle;nginx 直接读目录,不用重启。

</details>

<details>
<summary><b>挂在已有 Traefik 后面 —— HTTPS,不占宿主端口</b></summary>

`deploy/docker-compose.yml` 是权威生产栈:label 路由 + Let's Encrypt,
拉已发布镜像而非本地构建。用 `MICA_VERSION` 钉版本。
见[部署文档](docs/deploy.md#behind-traefik-the-canonical-production-stack)。

</details>

**迁移是编译进 API 二进制的,启动时自动跑**——没有单独的迁移步骤。

完整说明:[部署](docs/deploy.md) · [备份](docs/backup.md) · [发版流程](docs/release.md)

## 🧩 仓库结构

```
crates/
  api-server     Axum HTTP + WebSocket;迁移;唯一和 Postgres/S3 打交道的地方
  app-core       文档操作、同步、snapshot↔yrs 桥接
  mica-core      CRDT 文档(yrs)—— from_blocks / to_blocks
  markdown       Markdown 引擎:块模型、解析、渲染(权威;Dart 侧是镜像)
  interchange    压缩包级别的导入导出规划 —— 纯逻辑,无 I/O
  mcp-server     架在 REST API 上的 MCP 工具面
  cli            mica-cli
  infra          共享基础设施
clients/mica_flutter/
  lib/editor     自绘编辑器(render.dart 是那块画布)
  lib/local      本地 vault 存储(经 flutter_rust_bridge 调 Rust)
  rust           客户端侧 Rust 核心(FFI)
docs/            设计文档
migrations/      Postgres schema,API 启动时自动应用
```

## 🛠 开发

需要 Rust(stable,见 `rust-toolchain.toml`)、Flutter、Docker。
一切都走 [`just`](https://github.com/casey/just),`just --list` 看全部 recipe。

```sh
cp .env.example .env
just dev             # 整套后端进 Docker 并灌好种子(首次要编译)
just app             # Flutter 桌面客户端
just app chrome      # Flutter web 客户端 —— 自动连 :8080 的后端
just dev-down        # 全停
just test            # cargo test + flutter test
just check           # fmt + clippy + analyze
```

不配 `S3_*` 时文件相关端点返回 `503`,其余功能正常;`ANTHROPIC_API_KEY` 同理。

**动任何结构性的东西之前,先读 [`CLAUDE.md`](CLAUDE.md)**(项目原则、不变量、发版流程)
和 [`docs/lessons.md`](docs/lessons.md)——后者记录了那些不变量被破坏时的实际代价。

三条不看就会被咬的规则:

1. **Markdown 语法是故意两端各写一份的**(Rust 引擎 + Dart 镜像,因为输入规则不可能
   每敲一个键走一次网络)。两边由共享一致性 fixture 钉住——改一边,两套测试都要跑。
2. **每个 bug 修复都配回归测试**,提交信息写根因,不写流水账。
3. **新渲染能力先抽机制**,不许往 `render.dart` 堆 `if` 分支。
   见[渲染架构](docs/render-architecture.md)。

## 📚 文档

| | |
| --- | --- |
| [架构](docs/architecture.md) | 系统形状和背后的决策 |
| [踩过的坑](docs/lessons.md) | 代价最大的那些 bug,以及为什么 |
| [编辑器设计](docs/editor.md) · [引擎](docs/editor-engine.md) · [渲染](docs/render-architecture.md) | 编辑器的原则与内部实现 |
| [同步与 API](docs/sync-and-api.md) | REST 接口面和 WebSocket 消息封套 |
| [导出/导入](docs/export-import.md) | 归档格式、Notion 适配、round-trip 规则 |
| [CommonMark 记分牌](docs/commonmark-scoreboard.md) | 规范一致性,逐版本跟踪 |
| [本地优先](docs/local-first-plan.md) · [Vault 模式](docs/vault-mode.md) | 本地世界 |
| [MCP 服务](docs/mcp-server.md) · [接入客户端](docs/mcp-connect.md) | AI 工具访问 |
| [新机搭建](docs/bootstrap.md) | 一台干净的 Windows 机器,从零到能开发 |
| [部署](docs/deploy.md) · [发版](docs/release.md) · [备份](docs/backup.md) | 怎么跑起来 |
| [快捷键](docs/shortcuts.md) | 权威快捷键清单 |
| [路线图](docs/roadmap.md) | 接下来做什么 |

设计文档之间有出入时,**以 `CLAUDE.md` 和它指向的文档为准**。

## 🤝 参与贡献

欢迎 issue 和 PR。小修小补之外的改动,**请先开个 issue 讨论**——
这个代码库有相当一部分是以不明显的方式承重的,`docs/lessons.md` 就是为此而生的。

## 📄 许可证

[AGPL-3.0-or-later](LICENSE)。
