# Mica 桌面端技术方案与开发计划

> 2026-06-06 定稿。新会话(尤其换机器后)从这里恢复上下文:先读本文 + CLAUDE.md,再看 `git log --oneline -30`。

## 技术选型(已决策)

**Flutter Desktop 原生**。一份 Flutter 代码库 → web(现状不动)+ 桌面原生编译。否决 Tauri/Electron 壳。

- 参考 AppFlowy 桌面架构(Flutter UI + Rust 核心 FFI),但**不付**他们的 React web 重写税——Mica 的 Flutter web 已可用
- AFFiNE 从 Tauri 迁 Electron 的实例证明 webview 一致性(尤其 Linux webkit2gtk)是生产级的坑;CanvasKit-in-webview 性能更敏感
- Tauri 的核心卖点(Rust 进程内嵌)用 Dart FFI 同样可得,AppFlowy 五年验证

**两阶段**:

1. **Phase 1 云端入口**:桌面客户端连现有 API,数据在服务端
2. **Phase 2 离线优先**:mica 核心 crate 编译成库,Dart FFI 进程内嵌入(AppFlowy rust-lib 模式),本地 SQLite + 同步引擎

**目标矩阵(6 端,一份代码)**:web(已上线)、Linux、Windows、macOS、iOS、Android。
桌面包必须各自系统构建(无跨平台编译),CI 用 GitHub Actions 三 OS runner。移动端的增量成本在触屏交互适配(现有编辑器是鼠标中心:hover 工具条、树 hover 浮现箭头、拖拽手柄),不动架构。

## Windows 优先 — Phase 1 计划

| 里程碑 | 内容 | 待决 |
|---|---|---|
| **M1 跑起来** | `flutter create --platforms=windows .`;7 个 stub IO 化;连现有云端 API | token 存储(明文 vs DPAPI) |
| **M2 像桌面应用** | 窗口大小/位置记忆、最小尺寸、快捷键;**中文 IME 专项验证**(自绘编辑器 + TextInputClient,Windows 候选窗定位与 web 路径完全不同) | 标题栏:系统原生 vs 自定义无边框 |
| **M3 可分发** | mermaid 后端渲染端点(Rust 出 SVG/PNG,六端共享);安装包 | 分发形态:MSIX/winget vs Inno vs 绿色 zip;自动更新 |

**开发模式**:日常开发可留在 Linux(代码 99% 平台无关;Linux 工具链已装好,`flutter build linux` 已验证一次通过),Windows 机用于出包与平台特调。

### M1 状态(2026-06-06,核心闭环)

- Windows 工具链就位:Flutter 3.44.1(`C:\flutter`,已入 PATH)、VS Build Tools 2026 + Win10 SDK(本机已有)。`windows/` 已脚手架,debug 构建通过,176 测试全过。
- prefs / clipboard / open_url 三个 stub 已 in-house IO 化(见上表);pick_file/pick_image/image_actions 暂缓 M2。
- 修了 `_resolveBaseUri()` 非 web 下的坏 `file://` base(改默认 `http://127.0.0.1:8080`,`--dart-define=MICA_API_BASE_URL` 可覆盖)。
- **连 API 端到端验证通过**:本机后端 = `docker compose up -d --build postgres api`(**Docker Desktop**,端口原生发布到 Windows `127.0.0.1`)。桌面端默认构建(base `127.0.0.1:8080`)即可连:autologin→自动注册 demo(migrations 无 seed,`_devAutoLogin` 登录失败自动 register)→列 workspace(api 日志 login 401→register 200→workspaces 200)。
- **网络方案(已定)**:用 **Docker Desktop** 跑栈,容器端口直达 Windows localhost,无需 dart-define、无需 WSL IP。备选:WSL 内原生 docker 时本机 localhost 转发不生效,需连 WSL eth0 IP(随重启变)或 `--dart-define=MICA_API_BASE_URL=http://<wsl-ip>:8080`。
- 辅助工具(MCP / code-review-graph / skills)重配清单见 `docs/dev-environment.md`。

### 待用户拍板(M2/M3 前)

1. 标题栏风格:原生(省事)vs 无边框自定义(Notion/AFFiNE 风,Windows 上拖拽/snap 细节坑多)
2. 分发形态与是否要自动更新

## Stub → IO 清单(Phase 1 核心工作量)

桌面构建走 `*_stub.dart` 变体(条件导入已就位,**编译已验证通过**),逐个实现:

| 文件 | 现状 | 桌面实现 |
|---|---|---|
| `lib/prefs_stub.dart` | ✅ 已实现 | 本地 JSON `%APPDATA%/mica/prefs.json`(其他 OS 走 XDG/Library),同步读写契约不变 |
| `lib/editor/clipboard_copy_stub.dart` | ✅ 已实现 | 框架 `Clipboard`(无插件) |
| `lib/editor/open_url_stub.dart` | ✅ 已实现 | `Process` → `explorer.exe`/`open`/`xdg-open`(无 url_launcher) |
| `lib/editor/rich_paste_stub.dart` | no-op | 同上(HTML 粘贴可后置) |
| `lib/editor/pick_file_stub.dart` | ⏸ 暂缓 M2 | 桌面文件选择器(依赖 file_selector vs 自写 Win32 通道,待决) |
| `lib/editor/pick_image_stub.dart` | ⏸ 暂缓 M2 | 同上 |
| `lib/editor/image_actions_stub.dart` | ⏸ 暂缓 M2 | 存盘 + 图片剪贴板(框架不内置图片剪贴板) |
| `lib/editor/mermaid_preview_stub.dart` | no-op | **不做客户端实现** → 后端渲染端点(M3) |

math 公式(flutter_math_fork)纯 Flutter,桌面直接可用,无需处理。

## 遗留事项(不阻塞桌面端)

- ~~Markdown P2~~ 已闭环(2026-06-06):CommonMark 641/641 = 100%,GFM 24/24,见 docs/commonmark-scoreboard.md
- 渲染架构 deferred(docs/render-architecture.md):`_NodeLayout` 字段收敛进 rendererData;hit-test 走 renderer 分发——下个新块类型进来时顺手做

## 环境备忘(Linux 开发机)

- `just dev-web`:构建 web bundle(末尾自动 chmod);dev nginx 已配 `Cache-Control: no-store`(防 stale bundle,这个坑栽过三次)
- 验证流:改完 → `flutter test`(目前 176 个)→ `just dev-web` → playwright-cli 实测截图;**开测前确认没有旧标签页连着同一文档**(幽灵会话曾污染过协同数据,杀法:`playwright-cli kill-all` + 查 `ps aux | grep chrome`)
- DB 取证:`docker exec mica-postgres psql -U mica -d mica`;操作流在 `document_updates`(payload->operations),快照在 `document_snapshots`(payload->blocks,字段名 `type`)
- 代码字体:Roboto Mono 已打包(`fonts/`);web 上 `'monospace'` 族名不解析,新代码一律用 `kMonoFont`(model.dart)
