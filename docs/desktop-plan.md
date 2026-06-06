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

### 待用户拍板(M2/M3 前)

1. 标题栏风格:原生(省事)vs 无边框自定义(Notion/AFFiNE 风,Windows 上拖拽/snap 细节坑多)
2. 分发形态与是否要自动更新

## Stub → IO 清单(Phase 1 核心工作量)

桌面构建走 `*_stub.dart` 变体(条件导入已就位,**编译已验证通过**),逐个实现:

| 文件 | 现状 | 桌面实现 |
|---|---|---|
| `lib/prefs_stub.dart` | 内存 Map(重启即丢) | 本地 JSON 文件(`~/.config/mica/` 或 `%APPDATA%`) |
| `lib/editor/clipboard_copy_stub.dart` | no-op | dart:io / 平台通道 |
| `lib/editor/rich_paste_stub.dart` | no-op | 同上(HTML 粘贴可后置) |
| `lib/editor/pick_file_stub.dart` | no-op | 桌面文件选择器 |
| `lib/editor/pick_image_stub.dart` | no-op | 同上 |
| `lib/editor/open_url_stub.dart` | no-op | 系统默认浏览器 |
| `lib/editor/image_actions_stub.dart` | no-op | dart:io 下载/保存 |
| `lib/editor/mermaid_preview_stub.dart` | no-op | **不做客户端实现** → 后端渲染端点(M3) |

math 公式(flutter_math_fork)纯 Flutter,桌面直接可用,无需处理。

## 遗留事项(不阻塞桌面端)

- Markdown P2:转义、自动链接(CommonMark 还差 ~23 个失败例,见 crates 测试)
- 渲染架构 deferred(docs/render-architecture.md):`_NodeLayout` 字段收敛进 rendererData;hit-test 走 renderer 分发——下个新块类型进来时顺手做
- "容器介绍"页有测试残留(`dfssdf` 行、一个被降级的 H2,行首敲 `## ` 可恢复)

## 环境备忘(Linux 开发机)

- `just dev-web`:构建 web bundle(末尾自动 chmod);dev nginx 已配 `Cache-Control: no-store`(防 stale bundle,这个坑栽过三次)
- 验证流:改完 → `flutter test`(目前 176 个)→ `just dev-web` → playwright-cli 实测截图;**开测前确认没有旧标签页连着同一文档**(幽灵会话曾污染过协同数据,杀法:`playwright-cli kill-all` + 查 `ps aux | grep chrome`)
- DB 取证:`docker exec mica-postgres psql -U mica -d mica`;操作流在 `document_updates`(payload->operations),快照在 `document_snapshots`(payload->blocks,字段名 `type`)
- 代码字体:Roboto Mono 已打包(`fonts/`);web 上 `'monospace'` 族名不解析,新代码一律用 `kMonoFont`(model.dart)
