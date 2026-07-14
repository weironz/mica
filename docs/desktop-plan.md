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

> **2026-06-06 优先级(别搞错)**:桌面端**主场 = 在线模式**(连云端 API,与 web 同),**先做扎实**——即 Phase 1 收尾 + M2/M3。**纯离线只是一种场景,属 Phase 2(CRDT,future)**,不是当前开发重点。
> Phase 2 的完整设计(yrs/SQLite/bigserial 同步/frb v2/本地身份/对象存储/里程碑/红线)已深入调研 AppFlowy + AFFiNE 后**存档**在 `docs/phase2-offline-crdt.md`。
>
> **2026-06-08 更新(收官)**:Phase 1(M1–M3,在线)+ Phase 2(M4 云同步 / M5 对象存储 / polish:流截断·回滚 UI·协同光标·split-join 定案·blob 离线镜像·本地→云迁移)**均已完成**。桌面端核心开发收官,`v0.1.0` 安装包已发。剩余皆非阻塞,见文末「遗留事项」。

**目标矩阵(6 端,一份代码)**:web(已上线)、Linux、Windows、macOS、iOS、Android。
桌面包必须各自系统构建(无跨平台编译),CI 用 GitHub Actions 三 OS runner。移动端的增量成本在触屏交互适配(现有编辑器是鼠标中心:hover 工具条、树 hover 浮现箭头、拖拽手柄),不动架构。

## 技术路线定稿(架构分层与依赖边界,2026-06-06)

### 三层架构 — 各层用各自标准语言,没有「额外引入」的编程语言

| 层 | 语言 | 位置 | 职责 |
|---|---|---|---|
| 数据/服务面 | **Rust** | `crates/` | 后端服务;Phase 2 编译成库走 Dart FFI |
| 客户端 UI | **Dart / Flutter** | `clients/mica_flutter/lib/` | 一份代码 6 端,**所有业务逻辑在此** |
| 各平台原生宿主壳 | 各 OS 语言 | `windows/`·`linux/`=C++,`macos/`·`ios/`=Swift/ObjC,`android/`=Kotlin/Java,web=JS 引导 | `flutter create` 生成的标准样板:开窗口、塞引擎、注册插件。**不写业务逻辑** |

- `windows/runner/` 的 C++ 是 Flutter **强制**的平台宿主壳,**不是新增的开发语言、也不是第三方依赖**;改它仅限平台粘合的几行,本项目尽量不碰。
- 平台特定 Dart 能力走**条件导入**(`xxx.dart` → `export 'xxx_stub.dart' if (dart.library.html) 'xxx_web.dart'`):桌面/移动用 dart:io 变体,web 用 web 变体;桌面专属包(如 window_manager)靠此**不进 web bundle**。

### 依赖与 in-house 边界(对原则 #1 的校准)

- **in-house 自研**只留给**核心数据面**:CRDT、文档模型、同步引擎、自绘编辑器——值得自研、是护城河。
- **标准边角/平台功能**(窗口管理、文件对话框、URL 打开…)用成熟社区包或框架内置:自研要背 N 套平台原生层维护,违背原则**初衷**(省心可控)而非贴合它。
- **依赖豁免清单**(新增需在此登记 + 理由):`flutter_math_fork`(数学公式)、`window_manager`(桌面窗口大小/位置/最小尺寸)。
  - ⚠️ window_manager 已挂迁移公告 → `libnativeapi/nativeapi-flutter`。但继任者 v0.1.x「Work in Progress」、缺 bounds/min-size/resize-move 监听等我们要的 API,**暂不迁**。已靠 `window_setup.dart` 条件导入隔离,待其出稳定版 + 补齐 API 后评估迁移,**成本约一个文件**。
- 已 in-house 零依赖实现:`prefs`(JSON 持久化)、`clipboard_copy`(框架 Clipboard)、`open_url`(Process)。

### M2 落定方案

- **标题栏 = 系统原生**(否决无边框自定义:要写大量 Win32 拖拽/snap 或更重依赖,M2 不值当)。
- **窗口管理(最小尺寸 + 大小/位置记忆)= `window_manager` 包**(Dart),条件导入隔离;窗口边界存 prefs JSON(键 `windowX/Y/Width/Height`)。否决「纯 C++ 注册表自研」(属平台边角功能)。
- **中文 IME** = 验证既有 `TextInputClient` 路径在 Windows 的候选窗定位/组合态/commit;修 `render.dart caretRectFor()` 漏算文档级滚动导致候选窗错位的 bug。
- **快捷键** = 顶层 Shortcuts/Actions 加 App 级快捷键,不与编辑器热路径(editor.dart `_onKey`)冲突。
- **前置**:Windows 装插件需开**开发者模式**(插件用符号链接);详见 `docs/dev-environment.md`。

## Windows 优先 — Phase 1 计划(M1–M3 ✅ 全部完成)

| 里程碑 | 内容 | 状态 |
|---|---|---|
| **M1 跑起来** | `flutter create --platforms=windows .`;7 个 stub IO 化;连现有云端 API | ✅(token 存储:明文 JSON,DPAPI 留作后续) |
| **M2 像桌面应用** | 窗口大小/位置记忆、最小尺寸、快捷键;**中文 IME 专项验证**(自绘编辑器 + TextInputClient);文件对话框 + 富剪贴板 + IME 组合区(批次1/2/3) | ✅(标题栏=系统原生 + window_manager) |
| **M3 可分发** | mermaid 客户端 merman Dart FFI(离线);Inno 安装包 + GitHub release `v0.1.0` | ✅(自动更新/签名:见下决策,均先不做) |

**开发模式**:日常开发可留在 Linux(代码 99% 平台无关;Linux 工具链已装好,`flutter build linux` 已验证一次通过),Windows 机用于出包与平台特调。

### M1 状态(2026-06-06,核心闭环)

- Windows 工具链就位:Flutter 3.44.1(`C:\flutter`,已入 PATH)、VS Build Tools 2026 + Win10 SDK(本机已有)。`windows/` 已脚手架,debug 构建通过,176 测试全过。
- prefs / clipboard / open_url 三个 stub 已 in-house IO 化(见上表);pick_file/pick_image/image_actions 暂缓 M2。
- 修了 `_resolveBaseUri()` 非 web 下的坏 `file://` base(改默认 `http://127.0.0.1:8080`,`--dart-define=MICA_API_BASE_URL` 可覆盖)。
- **连 API 端到端验证通过**:本机后端 = `docker compose up -d --build postgres api`(**Docker Desktop**,端口原生发布到 Windows `127.0.0.1`)。桌面端默认构建(base `127.0.0.1:8080`)即可连:autologin→自动注册 demo(migrations 无 seed,`_devAutoLogin` 登录失败自动 register)→列 workspace(api 日志 login 401→register 200→workspaces 200)。
- **网络方案(已定)**:用 **Docker Desktop** 跑栈,容器端口直达 Windows localhost,无需 dart-define、无需 WSL IP。备选:WSL 内原生 docker 时本机 localhost 转发不生效,需连 WSL eth0 IP(随重启变)或 `--dart-define=MICA_API_BASE_URL=http://<wsl-ip>:8080`。
- 辅助工具(MCP / codebase-memory-mcp / skills)重配清单见 `docs/dev-environment.md`。

### ~~待用户拍板(M2/M3 前)~~ 已拍板(2026-06-07)

1. 标题栏风格:**定系统原生**(否决无边框自定义,见「技术路线定稿」)
2. 分发形态:**定 Inno per-user 安装包 + GitHub release**;**自动更新先不内置**、**代码签名跳过**(见「分发」决策)

## Stub → IO 清单(Phase 1 核心工作量)

桌面构建走 `*_stub.dart` 变体(条件导入已就位,**编译已验证通过**),逐个实现:

| 文件 | 现状 | 桌面实现 |
|---|---|---|
| `lib/prefs_stub.dart` | ✅ 已实现 | 本地 JSON `%APPDATA%/mica/prefs.json`(其他 OS 走 XDG/Library),同步读写契约不变 |
| `lib/editor/clipboard_copy_stub.dart` | ✅ 已实现 | 框架 `Clipboard`(无插件) |
| `lib/editor/open_url_stub.dart` | ✅ 已实现 | `Process` → `explorer.exe`/`open`/`xdg-open`(无 url_launcher) |
| `lib/editor/rich_paste_stub.dart` | ✅ 已实现(批次2c) | HTML 富文本粘贴(pasteboard 富剪贴板) |
| `lib/editor/pick_file_stub.dart` | ✅ 已实现(批次1) | 桌面文件对话框(file_picker,豁免#3) |
| `lib/editor/pick_image_stub.dart` | ✅ 已实现(批次1) | 同上 |
| `lib/editor/image_actions_stub.dart` | ✅ 已实现(批次2a/2b) | 存盘 + 图片剪贴板读写(pasteboard,豁免#4) |
| `lib/editor/mermaid_preview_stub.dart` | ✅ 已实现 | **客户端 merman Dart FFI**(纯 Rust 引擎)→ SVG → css 内联 → flutter_svg 栅格,离线;见下「mermaid 桌面渲染」 |

math 公式(flutter_math_fork)纯 Flutter,桌面直接可用,无需处理。

### mermaid 桌面渲染(M3,2026-06-07 完成)

**方案(调研 AppFlowy/AFFiNE 后定;详见 CLAUDE.md 原则6 那次教训)**:纯客户端、离线、跨平台一套数据逻辑,无 webview、无后端、无 JS 引擎。

链路(`mermaid_preview_stub.dart`,仅非 web):
1. **merman**(豁免#5,纯 Rust headless mermaid 引擎,FFI,pub 包自带各平台原生库)`Merman.open().renderSvg(src, pipeline:resvg-safe)` → SVG。
2. **`mermaid_svg_inline.dart`**(自研 CSS 内联后处理器,用 xml 解析):merman 把主题放在带后代选择器的 `<style>` CSS 里(`#merman .node rect{fill:..}`),而纯 Dart 渲染器都不解析 CSS;把规则拍平成元素 `style` 属性,删 `<style>`/`<marker>`,并把 flutter_svg 会抛错的 `font-weight:bolder/lighter` 归一成 `bold/normal`。merman 文档明确把 inline-styling 列为 **host 边界**(Zed 集成同此),这是预期接缝不是 hack。
3. **flutter_svg**(豁免#6)栅格化 → ui.Image,按 web 同款「2×目标宽、clamp」缩放;失败静默返回 null → 块保留高亮源码(降级)。

**箭头**:flutter_svg 不渲 `<marker>`,所以内联器**自己合成箭头**(`_synthesizeArrowheads`):解析每条边(`<path d>` 或 `<line>`)的端点与切线方向,把 marker 的子图形克隆进一个 `<g transform="translate(端点) rotate(方向) scale(viewBox→markerW) translate(-refX,-refY)">`,再删 marker/defs。flowchart/sequence/state/class 箭头均正确。
**实测**(Windows runner 集成测试 `integration_test/mermaid_render_test.dart`,带 raster-ink 断言):flowchart / sequence / class / state / pie / gantt 六类均正确渲染主题色、文字、虚实线、**箭头**,离线可用。
**已知小差异**:class 继承三角是实心(real mermaid 为空心,marker fill 默认 #333);merman 仍 0.x alpha,已锁版本。Web 端保持原 mermaid.min.js 路径不变。

### 分发(M3,2026-06-07)

- **打包**:Inno Setup per-user 安装包(`installer/mica.iss`,`PrivilegesRequired=lowest`,无 UAC)。`SourceDir` 指向 `build/windows/x64/runner/Release`。`AppVersion` 用 `#ifndef`,支持 `ISCC /DAppVersion=x.y.z` 覆盖。安装包 ~16.5MB(merman_ffi.dll 13MB 占大头)。
- **首份 release**:`v0.1.0` 已发布(`gh release create`,prerelease,asset `Mica-Setup-0.1.0.exe`)。标签**故意指向无 workflow 的 commit**(`5c47bd9`)以免触发 CI 二次构建。
- **CI**:`.github/workflows/release-windows.yml`(workflow 名现为 **Release**,不止 Windows),push `v*` 标签触发两组 job:① `windows`——windows runner 上 `flutter build windows --release` → choco 装 Inno → ISCC → softprops 发布安装包(带 `make_latest`,喂 in-app 更新器);② `cli`(2026-07-13 加)——`needs: windows`,matrix 在 windows/linux/macos 上 `cargo build --locked -p mica-cli --release`,把 `mica-cli-<版本>-<os>-<arch>` 二进制挂到同一 release。**`needs: windows` 是刻意的**:让安装包(+make_latest)先落地,避免更新器读到 `/releases/latest` 时安装包资产还没挂上的空窗;cli 用 rustls-tls 纯 Rust 无 OpenSSL,Linux/macOS 无系统依赖。⚠️ **注意**:tag 触发用的是**标签 commit 处**的 workflow 文件——未来打 tag 的 commit 必须含此 workflow;`workflow_dispatch` 要等 workflow 进默认分支才在 UI 出现。
- **决策(2026-06-07,用户拍板)**:
  - **自动更新:~~先不内置~~ → 已内置(2026-07-10,自研方案 B)**。当初调研结论是抄 AppFlowy 的 `auto_updater`(WinSparkle)+ appcast,但重(第三方框架 + CI 生成 appcast + DSA 密钥);实现时改走更轻的自研路:`lib/updater*.dart` 查 GitHub Releases API 拿最新 tag + `Mica-Setup-*.exe` 链,下载后 `Process.start` 拉起 Inno 安装器 `/VERYSILENT /CLOSEAPPLICATIONS`(强制关掉当前程序→装→由 `mica.iss` 的 `[Run]` 重启;为此 `[Run]` 去掉了 `skipifsilent`,对 **v0.1.6+** 安装包生效)。入口在 About(`UpdateChecker`,仅 `updateSupported`=Windows 显示,条件导入使 `dart:io` 不进 web bundle)。**取舍:无 DSA 更新签名**,信任 = 从 GitHub 走 HTTPS(与"程序未签名"一致)。要更强校验再上 WinSparkle。
  - **代码签名:跳过**——未签名,首次运行有 SmartScreen 警告(More info→Run anyway,同 AppFlowy)。证书要 CA 签发,开源免费正路是 SignPath Foundation(需项目所有者申请),日后接进 Inno `SignTool` 指令。

## 遗留事项(均不阻塞,2026-06-08 复核)

- ~~Markdown P2~~ 已闭环(2026-06-06):CommonMark 641/641 = 100%,GFM 24/24,见 docs/commonmark-scoreboard.md
- **§7 离线态上行 blob differ**(云端断网插图→重连上传改写):Phase 2 窄边角增强,主离线面=本地模式已全覆盖、迁移期 blob 上行已实现。设计见 `phase2-offline-crdt.md §7.1`。
- ~~**CI release pipeline 未实跑验证**~~ **已首验(2026-06-08)**:给 workflow 加了 dry-run(`workflow_dispatch` 的 `publish` 开关,默认 false → 只 build+package+传 artifact、不发 release)。dispatch `version=0.1.1-rc publish=false` 在 Windows runner 上**全绿**:flutter build windows release → 装 Inno → ISCC 打包 → 产出 `Mica-Setup-0.1.1-rc.exe`(17.9MB)artifact,publish 步骤正确 skip。**唯一未实跑** = `softprops` 发布上传(dry-run 跳过,成熟 action,tag 推送会走到)。正式发版:推 `v*` tag 或 dispatch `publish=true`。
- **window_manager → nativeapi-flutter 迁移**:watch list,卡上游 v0.1.x WIP,暂不迁(成本约一个文件,见「技术路线定稿」依赖边界)。
- **渲染架构 deferred**(docs/render-architecture.md):`_NodeLayout` 字段收敛进 rendererData;hit-test 走 renderer 分发——下个新块类型进来时顺手做。
- **token 存储**:当前明文 JSON;DPAPI 加密留作后续(M1 待决项,非阻塞)。

## 环境备忘(Linux 开发机)

- `just dev-web`:构建 web bundle(末尾自动 chmod);dev nginx 已配 `Cache-Control: no-store`(防 stale bundle,这个坑栽过三次)
- 验证流:改完 → `flutter test`(目前 176 个)→ `just dev-web` → playwright-cli 实测截图;**开测前确认没有旧标签页连着同一文档**(幽灵会话曾污染过协同数据,杀法:`playwright-cli kill-all` + 查 `ps aux | grep chrome`)
- DB 取证:`docker exec mica-postgres psql -U mica -d mica`;操作流在 `document_updates`(payload->operations),快照在 `document_snapshots`(payload->blocks,字段名 `type`)
- 代码字体:Roboto Mono 已打包(`fonts/`);web 上 `'monospace'` 族名不解析,新代码一律用 `kMonoFont`(model.dart)
