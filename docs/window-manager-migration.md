# window_manager → nativeapi 迁移评估(2026-07-23)

**结论:现在不迁,继续用 `window_manager` 0.5.2(+ `tray_manager` / `screen_retriever`)。**

起因:window_manager 的 README 挂了迁移通知——"This plugin is being migrated to
`libnativeapi/nativeapi-flutter`"(基于统一 C++ 核心 `libnativeapi/nativeapi`)。担心后续大量重构,
故按项目原则 #6 扒了 nativeapi 的真实现状再拍板。

## 一句话

一个**硬阻断** + 一堆**不成熟信号**锁死决策;而留在 0.5.2 几乎零成本。迁移通知是"方向声明",**不是 EOL 日期**。

## 🚫 硬阻断:nativeapi 现在无法拦截关闭按钮

mica 的整个窗口层(`clients/mica_flutter/lib/window_setup_desktop.dart`)是**围绕接管 X 按钮**建的:
`setPreventClose(true)` + `onWindowClose` → 关闭时问"退出 / 最小化到托盘",并在 `exit(0)` 前 flush
本地状态(见该文件 `_quitNow` / `_CloseHandler`)。nativeapi 今天做不到:

- **没有 `setPreventClose`、没有可否决的 close hook。**
- 有 `WindowClosedEvent`,但那是**关闭之后**的通知,取消不了;有 `isClosable`(直接禁用 X,但不能跑代码)。
- 最说明问题:它有 `setWillShowHook` / `setWillHideHook`(都可否决),**唯独没有 `setWillCloseHook`**——
  可否决 hook 模式哪都有,就是没给 close。

→ "关闭 → 托盘 / 先 flush 再退"**无法在 nativeapi 上复刻**。单这一条即毙。

## 🚧 不成熟信号(次要但一致)

- nativeapi(flutter 包)pub.dev **0.1.4**(~2026-06 发),自标 **"🚧 Work in Progress … under active
  development"**;pub.dev 还**落后于它自己那个不完整的 `main`**(仓库 issue #2 在问何时发布新状态)。
- **mica 正好用到的原语最近都出过 bug**:窗口事件监听 macOS 不触发(#12 open)、titleBarStyle glitch +
  缺 `skipTaskBar`(#13,作者原话想"补齐 window_manager api")、Windows `setMinimumSize` 返回 0×0(#9)、
  `setBounds` 宽高传反(#33)、Windows 托盘菜单回调坏过(#3/#4/#5)。
- 平台"全 ✅"是**目标不是实测**:Linux `getCurrent()` 崩(#6 open)、macOS 监听回调死(#12)、Android 曾
  因缺符号构建失败(#1)。
- **没有 window_manager → nativeapi 迁移指南**,箭头只是意图,没给消费者搭桥。

## API 覆盖(mica 用面 vs nativeapi)

mica 的三家用面**全集中在 `window_setup_desktop.dart` 一个文件**(+ `window_snapped_win.dart` 的 snap
检测),main.dart 只调一个 `initDesktopWindow()`。

| mica 用到 | nativeapi 今天 |
|---|---|
| `setPreventClose` + `onWindowClose`(接管 X) | **无**(硬阻断,见上) |
| `ensureInitialized` / `waitUntilReadyToShow` / `WindowOptions` / `TitleBarStyle.normal` | 无对应,改用 `getCurrent()` 模型——init 要重写 |
| 窗口事件 `onWindowResized/Moved/Maximize/Unmaximize` | 有 `Resized/Moved/Maximized/Restored` 事件,但走 per-object `EventEmitter`(非 `addListener(WindowListener)`)——**重写非改名**,且 macOS 当前坏(#12) |
| 托盘 `tray_manager`(setIcon/ToolTip/ContextMenu/click) | 有 `TrayIcon` 等价物,但 API 形状不同(EventEmitter),近期 Windows 菜单 bug |
| 多屏 `screen_retriever.getAllDisplays()` → visiblePosition/Size | 有 `Display.workArea`(单 `Rect`,无分开的 visiblePosition/Size)——功能够,字段要重映射 |

"1 个文件"**不等于低风险迁移**:那个文件恰恰依赖 nativeapi 最缺的能力(close 拦截),且 init/监听/托盘
都是从头重写。

## ✅ 留下几乎零成本

- window_manager **0.5.2 ~2026-07 刚发**,在活跃维护、未归档(1.1k+ likes、无迫近的 Flutter 插件 API 破坏——
  18 天前发布的包按定义就是对着当前 stable 编的)。
- 我们 **pin 死 0.5.2**(见 `pubspec.yaml`),不会自己升级踩坑。

## 何时再评估(三个信号,全满足才动)

1. nativeapi-flutter 出**可用的可否决 close hook**(`setWillCloseHook` 在 Windows 能用)——**闸门能力**;
2. 它到 **~1.0 / 摘掉 "WIP"**,且 pub.dev 与 `main` 对齐;
3. 有**官方 window_manager → nativeapi 迁移指南**。

在此之前,迁移 = 拿一个能用、在维护的依赖去换一个做不了核心行为的 alpha。等它成熟再迁(~1 文件、成本低)
才是正解;现在迁反而更贵。

## 同类桌面栈对照(别人怎么做窗口)

"别人不依赖 window_manager" **≠ 有更好的 Flutter 替代**,纯粹是**技术栈不同**——它是 Flutter-only 的包,
只对 Flutter 桌面应用才适用。8 个同类里只有一个是 Flutter。

| 产品 | 桌面栈 | 窗口管理靠什么 |
|---|---|---|
| Notion / AFFiNE / SiYuan / Joplin / Logseq / Anytype | **Electron**(6 个) | Chromium 原生 `BrowserWindow` + `Tray` + `'close'` 事件拦截;尺寸恢复多用 `electron-window-state` |
| Outline | **纯 web**(无官方桌面端) | 浏览器 / PWA,不涉及原生窗口 |
| **AppFlowy** | **Flutter**(唯一,和 mica 同构 Flutter+Rust) | **`window_manager` ^0.4.3(主力)** + `bitsdojo_window`(仅 Windows 无边框标题栏);**无 tray_manager**(不做关闭到托盘)。还一起用了同家族的 `auto_updater` / `hotkey_manager`。 |

**结论:对 Flutter 桌面应用,依赖 `window_manager` 是正常、被同类验证过的选择。** 唯一的 Flutter 同类
AppFlowy 正是这么用的,且没找到"更好的 Flutter 替代";Electron 那 6 个用不到它,只因窗口控制是
Chromium 白送的。mica 在窗口行为上甚至比 AppFlowy 更全(AppFlowy 不做关闭到托盘,mica 用 tray_manager 做了)。

## 贡献 PR 推动 nativeapi 的工作量(若想主动推进 / 铺将来迁移的路)

维护者受欢迎(#13 作者原话"希望补齐 window_manager api",整套 hook seam 就是为这类功能预留的)。分两层:

| 目标 | 工作量 | 说明 |
|---|---|---|
| **只补 close hook**(那个硬阻断) | **一个周末**(~200–300 LOC / 8–10 文件) | 照抄现有 show/hide hook 骨架。**Windows 反而最简单**:已有 `WindowMessageDispatcher` 给 Flutter 窗口做子类化(现在就用它拦 `WM_GETMINMAXINFO`),其文档示例就是 `WM_CLOSE` 返回 0 否决。macOS = 镜像 swizzle `-performClose:`,Linux = `delete-event`。唯一设计点:close 的否决语义建议在核心里做成真正的 `bool` 返回,而非 show/hide 那套"suppress+replay"。 |
| **到 window_manager 平价**(mica 真能迁) | **数周(~3–6)** | 大头不是 close hook,而是**坏掉的窗口事件监听子系统**(#12):macOS delegate 回调体全被注释、Windows `StartEventListening` 是占位符 → resize/move/maximize/focus 桌面**根本不触发**;要三平台各自重连 + 跨系统实测。 |

**对 mica 的取舍:**
- **为 mica 自身别接全平价的活**:迁移当下买不到东西,平价是数周,且最大那块(事件子系统)要 macOS/Linux
  实测——mica 是 Windows 优先单人项目、CI 仅 windows-latest,测试负担不划算。
- **close hook 可作可选的"好公民"PR**:well-scoped、维护者接、正好解掉闸门能力,顺带对冲将来迁移的最大障碍。
  是利他 + 铺路,不是 mica 现在需要的。
- **别 drive-by 事件子系统**:要和维护者协调 + 三系统测试,超出周末 PR 范畴。

## 参照

- `libnativeapi/nativeapi`(C++ 核心)、`libnativeapi/nativeapi-flutter`(Dart 层):README(均标 WIP)、
  issues #1–#13(flutter)/ #3–#48(core)、on-disk Dart API(`window.dart` / `window_event.dart` /
  `window_manager.dart` / `tray_icon.dart` / `display.dart`);close hook 端到端 seam:`window_manager.dart`
  的 `setWillShowHook`、`src/capi/window_manager_c.cpp`、`src/window_manager.h`、三平台
  `src/platform/*/window_manager_*`、Windows `window_message_dispatcher.h`。
- pub.dev:`nativeapi`(0.1.4)、`window_manager`(0.5.2)。
- 同类:AppFlowy `frontend/appflowy_flutter/pubspec.yaml` + `lib/startup/tasks/windows.dart`;AFFiNE /
  SiYuan / Joplin / Logseq / Anytype 的 `package.json`(Electron);`outline/outline`(web)。
- 相关:CLAUDE.md 依赖豁免 #2(window_manager)/ #8(tray_manager);`docs/desktop-plan.md`。
