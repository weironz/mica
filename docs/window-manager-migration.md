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

## 参照

- `libnativeapi/nativeapi`(C++ 核心)、`libnativeapi/nativeapi-flutter`(Dart 层):README(均标 WIP)、
  issues #1–#13(flutter)/ #3–#48(core)、on-disk Dart API(`window.dart` / `window_event.dart` /
  `window_manager.dart` / `tray_icon.dart` / `display.dart`)。
- pub.dev:`nativeapi`(0.1.4)、`window_manager`(0.5.2)。
- 相关:CLAUDE.md 依赖豁免 #2(window_manager)/ #8(tray_manager);`docs/desktop-plan.md`。
