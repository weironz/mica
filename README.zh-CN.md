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

**只想用?** 直接拿[安装包](#-安装),这页别的内容你都不需要。

**想自托管、想改它、或者想看它怎么实现的?** 往下读。

> **项目状态**:活跃开发中,作者本人日常在用。迁移只向前跑、导出格式可无损往返,
> 所以放真实笔记是安全的。但这是 pre-1.0:**小版本之间不承诺兼容**,
> Linux/macOS 桌面版也还没发布。

## ✨ Mica 是什么

**两个世界,一个应用。** *本地*工作区就是你硬盘上的一个 Markdown 文件夹——不需要账号、
不联网、不同步。*云端*工作区跑在 Mica 服务端上,实时同步,可以分享。两者显式切换,
且**一次只显示一个**,所以永远不会搞不清一个页面到底存在哪。

**编辑器是画出来的,不是拼出来的。** 它是单个 Flutter `RenderBox`,自己绘制文字、光标、
选区和块,底层是 marks-over-plain-text 模型。不是控件树,不是 `WebView`,
也不是包了一层的第三方编辑器。

- **本地优先** —— 硬盘上的纯 Markdown,不需要账号。
- **实时协作** —— 基于 `yrs` 的 CRDT 同步,带协作者在线状态。
- **离线可用** —— 编辑排队、重连后收敛;图片等网络恢复再上传。
- **文件夹与页面** —— 拖拽排序和移动,工作区顺序按用户各自保存。
- **富块** —— 表格、代码高亮、LaTeX 公式、Mermaid 图、脚注。
- **版本历史** —— 自动快照 + 命名检查点,带 diff 预览。
- **导入** —— Markdown、文件夹、ZIP,以及 **Notion 导出包**。
- **导出** —— Markdown、HTML、PDF、ZIP;导出再导入无损。
- **公开链接** —— 只读分享单个文档。
- **MCP 服务** —— 让 Claude 读取、搜索、编辑你的笔记。
- **可选 AI** —— 没配 API key 时整个功能隐藏。

### Markdown 是存储格式,不是一个功能

底座是 CommonMark 0.31.2 —— **读侧 641/641 全过** —— 加 GFM(24/24),
再加一小层方言补 GFM 表达不了的东西:脚注、front matter、Pandoc 数学约定。
写侧输出规范化子集,**round-trip 是被 CI 地板钉住的不变量**。
见[记分牌](docs/commonmark-scoreboard.md)。

## 🤔 为什么又做一个

好用的笔记软件已经很多了。Mica 存在,是因为有一个组合它们都不完全满足:

- **[Obsidian](https://obsidian.md)** 把本地文件这件事做对了,但不开源,也没有真正的实时协作服务端。
- **[Notion](https://notion.so)** 编辑器做得好,但数据在人家数据库里。
- **[AFFiNE](https://github.com/toeverything/AFFiNE)** 和 **[AppFlowy](https://github.com/AppFlowy-IO/AppFlowy)** 都很优秀且开源——它们是 Mica 的参照系,本项目不止一次架构争论是靠读它们的源码定的。AFFiNE 是 web 优先,AppFlowy 在 Flutter 下面跑 Rust 核心。
- **[思源笔记](https://github.com/siyuan-note/siyuan)** 和 **[Logseq](https://github.com/logseq/logseq)** 更偏块模型那一端,Mica 更偏文档。

Mica 押的是**自绘编辑器 + Rust 数据面**:同一套文档模型既服务本地文件夹也服务同步服务端,
Markdown 是存储格式而不是导出目标。

**Mica 不是什么**:没有插件生态、没有移动端、没有白板和数据库视图,
打磨程度也远不及上面任何一个项目。如果你现在就需要这些,去用它们——是真的好用。

## 📦 安装

| 平台 | 下载 |
| --- | --- |
| **Windows** | [`Mica-Setup-*.exe`](https://github.com/weironz/mica/releases/latest) —— 应用会从同一来源自动更新 |
| **Linux / macOS 桌面版** | 尚未发布。Flutter 工程包含 Linux target 且代码与平台无关,但两者都没进 CI、没测过,请当作不支持。 |
| **Web** | 仅自托管,见下 |

**命令行** —— 每个 [release](https://github.com/weironz/mica/releases/latest) 都附带
Windows / Linux / macOS 三个平台的 `mica-cli`。它走同一套 API,并承载 MCP 服务
(`mica-cli mcp`)。见 [docs/cli.md](docs/cli.md)。

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

**迁移是编译进 API 二进制的,启动时自动跑**——没有单独的迁移步骤,
而且只支持向前滚。

完整说明:[部署](docs/deploy.md) · [备份](docs/backup.md) · [发版流程](docs/release.md)

## 🧩 怎么实现的

**Rust** 负责一切解析文件、遍历压缩包、做哈希、和存储打交道的活。
**Dart/Flutter** 负责绘制、光标与选区、命中测试,以及编辑器的延迟敏感热路径。
CRDT 同步用 `yrs`(Yjs 的 Rust 移植)—— Web 端用 Yjs 本体讲同一套线上格式,
所以是**一个权威引擎跑全平台**。

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
just dev-up          # Docker 里的 Postgres + MinIO
just dev-api         # cargo run -p mica-api-server(启动即跑迁移)
just app             # Flutter 桌面客户端
just app chrome      # Flutter web 客户端
just test            # cargo test + flutter test
just check           # fmt + clippy + analyze
```

不配 `S3_*` 时文件相关端点返回 `503`,其余功能正常;`ANTHROPIC_API_KEY` 同理。

**动任何结构性的东西之前,先读 [`CLAUDE.md`](CLAUDE.md)**(项目原则、被破坏过的不变量、
发版流程)和 [`docs/lessons.md`](docs/lessons.md)——后者记录了那些不变量被破坏时的实际代价。

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
| [部署](docs/deploy.md) · [发版](docs/release.md) · [备份](docs/backup.md) | 怎么跑起来 |
| [快捷键](docs/shortcuts.md) | 权威快捷键清单 |
| [路线图](docs/roadmap.md) | 接下来做什么 |

有些设计文档写在后来的决策之前——特别是 `architecture.md`,它成文时项目还是纯云端。
**以 `CLAUDE.md` 和它指向的文档为准。**

## 🤝 参与贡献

欢迎 issue 和 PR。小修小补之外的改动,**请先开个 issue 讨论**——
这个代码库有相当一部分是以不明显的方式承重的,`docs/lessons.md` 就是为此而生的。

## 📄 许可证

[AGPL-3.0-or-later](LICENSE)。
