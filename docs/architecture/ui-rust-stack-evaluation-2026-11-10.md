# 全平台全栈 Rust：UI 层迁到 GPUI 的可行性与技术栈选择（2026-11-10 调研）

> **状态：候选方向，未立项。** 本文是 [2026-11-01 的 GPUI 评估](ui-gpui-evaluation.md) 的续篇：那一篇只回答
> "桌面 + Web 能不能迁"，这一篇把**移动端**算进来，回答三个问题：
> 1. 现在迁过去可不可行；
> 2. 如果迁，要引入哪一套技术栈；
> 3. 除了 GPUI 还有哪些方案。
>
> **结论先行**：现在仍然不迁。如果将来迁，栈是 **gpui-kit（桌面 + Web）+ Native Shell（移动端）**，
> 两者同属 Longbridge、共用一套 Rust 组件思路；但移动端这一半今天还撑不起我们的自绘编辑器，
> 所以"全平台"目前做不到一步到位。立项前先过第 6 节的四道关卡。
>
> 文中的外部事实来自各项目主页、发布说明和搜索结果，**没有一项是我们自己跑出来的**；凡是没验证的都标了"未验证"。

## 1. 和上一篇相比，变了什么

| 项 | 2026-11-01 | 2026-11-10 |
| --- | --- | --- |
| 现役平台 | Windows 桌面 + Web | 加上 **Linux 桌面（.deb）和 Android（APK）**，0.14.4 起随 Release 发布；手机网页版也在持续修（gh#171、手指选择） |
| 客户端规模 | `lib/` 约 7.2 万行 Dart，编辑器约 2.5 万行，测试 288 个文件 | `lib/` 约 8.3 万行，编辑器约 2.6 万行，测试 315 个文件 |
| Rust 侧规模 | 未统计 | `crates/` 与桥接层合计约 10.7 万行 |
| 调研对象 | gpui-kit、一个社区 Markdown 编辑器 | 加上 **Native Shell（nsh）** 和 **gpui-fast** |
| "全平台"的含义 | 桌面 + Web | 桌面 + Web + **手机** —— 门槛明显变高 |

数据面仍然全部在 Rust（文档核心与 CRDT、Markdown 引擎、存储、导入导出、页树规则、接口类型），
所以迁移仍然只涉及 UI 层。

## 2. 三个概念各是什么

### 2.1 Zed 的 GPUI

Zed 编辑器自带的 Rust UI 框架，GPU 渲染、自己排版。

- **桌面**：macOS、Windows、Linux 是正式平台（Zed 本身三端发布；Zed 1.0 于 2026-04 发布）。
- **Web**：上游把"Zed on the web"列在"1.0 之后"的路线里，没有正式 Web 平台。
- **移动端**：上游没有 iOS / Android 支持，只有社区项目。
- **中文输入法**：Zed 仓库里有一条 Windows 上中文输入法重复上屏的问题单（#56327），原因是组字期间没有屏蔽按键消息。
  是否已修复，未验证。
- 上一篇把"GPUI 到 1.0"列为重新评估的信号。**Zed 到了 1.0，不等于 GPUI 这个库承诺了接口稳定**；
  GPUI 库自身的版本与稳定性承诺，本次未核实。

### 2.2 gpui-kit（longbridge）

基于 GPUI 的桌面应用框架，原 `gpui-component` 仓库现在就是它。约 2 万星，Apache-2.0，用于 Longbridge Pro。

- **分层**：`gpui-kit`（应用只依赖这一个 crate，转出 GPUI 与下面各层）、`gpui-base`（无样式的行为与基础设施）、
  `gpui-component`（带样式的组件）、`gpui-shell`（JS 扩展宿主，可选）。
- **平台**：macOS / Windows / Linux，以及浏览器（`wasm32-unknown-unknown`）。**不含移动端。**
- **和我们相关的组件**：输入框（单行、多行、自动增高）、代码编辑器（tree-sitter、LSP，自称 20 万行稳定）、
  Markdown / HTML 渲染、虚拟列表（含变高）、表格、Dock 布局、图表、AccessKit 无障碍、无头窗口 UI 测试。
- **Web 上的输入法**：官方 WebAssembly 指南说 Web 平台用一个隐藏的 HTML 输入框接收键盘和组字事件 ——
  和 Flutter Web 的做法一样。但有两个 PR（gpui-kit #3827、gpui-fast #108）在修"候选词窗口出现在页面角落而不是光标处"，
  其中一个注明没有在真实输入法会话里验证过。**Web 上的中文输入仍是未闭合的问题。**
- **桌面上的输入法**：一份 2026 年的 Rust GUI 实测综述把 GPUI Component 列进了输入法可用的一档（2026-08 补记）。
- 每个版本固定一份上游 GPUI 快照；示例用 `gpui-kit = "0.7"`。破坏性变更政策未见说明。

### 2.3 gpui-fast（longbridge）

GPUI 的性能分支：保留未变化的视图、布局和文字测量，滚动复用 GPU 贴片；还能把原生内容（如 WebView）嵌进窗口。
自称"实验性"，0.1.0，约 500 星。在 gpui-kit 里是一个可选开关，不改应用代码。**对我们是可选项，不是必选项。**

### 2.4 Native Shell（longbridge/nsh）

"写一次，各平台用原生控件渲染"的 UI 框架，Rust 核心，界面可以用 Rust 写，也可以用 TSX 写。约 2,300 星，Apache-2.0。

| 平台 | 渲染后端 |
| --- | --- |
| iOS（15+） | UIKit、CoreText |
| Android（8+） | Android 原生 View、`StaticLayout` |
| macOS / Windows / Linux | AppKit / Win32 + Direct2D / GTK 4；另有一个 GPUI 后端 |
| Web | HTML + CSS |
| 终端 | ratatui |

- 共用的是响应式引擎（Solid 风格的 signal）、flex / grid 布局、组件与状态；文字、输入、无障碍交给各平台自己的实现。
- **它和 GPUI 的关系**：GPUI 只是它的后端之一。最近的改动把 GPUI 后端降为"嵌入用"，不再支持
  `nsh package --target gpui` 独立打包；反过来 gpui-kit 可以嵌入 nsh 应用（gpui-kit #3601）。
- **成熟度**：发布说明能确认到 v0.3.0（主页同时出现 v0.4.0，未核实）；v0.3.0 带破坏性接口改名。
  v0.3.0 新增了"编辑器、表格、可停靠工作区、图表"的无样式基础组件。
- **对我们最关键的一点**：它是原生控件框架，主页没有描述自定义画布 / 自绘接口，也没有提富文本编辑和输入法。
  我们的编辑器是整块自绘画布，**能不能放进 nsh，今天没有证据**；能确认的只有"宿主应用可以注册原生组件"。

## 3. 如果切过去，引入哪一套栈

| 层 | 选择 | 说明 |
| --- | --- | --- |
| 桌面 UI（Windows / Linux / macOS） | **gpui-kit**（固定版本；`gpui-fast` 开关按性能实测决定） | 外壳、页树、对话框、设置直接用现成组件 |
| Web UI | **gpui-kit 编译到 wasm** | 与桌面同一套代码；能否成立取决于第 6 节的 Web 关卡 |
| 手机 UI（Android / iOS） | **Native Shell**，Rust 组件 | 目前唯一同一生态、有正式移动后端的选择；编辑器怎么放进去是未解问题 |
| 编辑器 | **自己写的 GPUI 元素**，直接调用 `mica-core` / `md-core` | 不能用现成的代码编辑器组件（那是纯文本代码编辑器）；输入法接入复用 `gpui-base` 输入框的那一层 |
| 文字与输入法 | GPUI 自带排版；Web 走隐藏输入框 | 中日韩输入是全栈风险最高的一处 |
| 无障碍 | AccessKit（gpui-kit 内置） | |
| 公式 | 已有的 Rust `ratex-*` | 不用新增 |
| 图片 / SVG | GPUI 自带的图片与 SVG 渲染 | Mermaid 目前靠 JS 出图，迁移后桌面端怎么跑要单独设计 |
| 托盘、文件选择、富文本剪贴板、窗口管理、自动更新 | 逐项选 Rust crate 或自己写 | 上一篇已列为"生态缺口"；每个新依赖按 AGENTS.md 逐项豁免 |
| 多语言 | 需要新选型 | 现在靠 Flutter 的 gen-l10n |
| 测试 | gpui-kit 的无头窗口测试 + 现有浏览器套件改造 | 315 个 Flutter 测试文件不能直接迁移 |

**可以删掉的**：Flutter / fvm、flutter_rust_bridge、Cargokit、生成的桥接代码、FFI 库指纹校验、
"Dart 里不许出现 Markdown 语法"一类门禁、Android 的 Gradle 工程（改由 nsh 生成）。

**这套栈的硬伤**：手机端和桌面端不是同一个 UI 框架。gpui-kit 与 nsh 共用的是语言、数据面和（部分）无样式组件，
**不是同一份界面代码**。外壳界面大概率要写两遍，编辑器在手机上怎么落地没有答案。
所以它是"全栈 Rust"，但还不是"一套界面代码跑全平台"——后者正是 Flutter 现在给我们的。

## 4. 其他方案

| 方案 | 做法 | 全栈 Rust | 手机 | Web | 中文输入法 | 对我们的主要问题 |
| --- | --- | --- | --- | --- | --- | --- |
| **A. 维持现状** | Flutter UI + Rust 数据面，继续把规则下沉到 Rust | 否 | 有（已发 APK） | 有 | 已踩平大部分坑 | 两种语言、桥接层和门禁要长期维护 |
| **B. gpui-kit（桌面 + Web），手机先用网页版** | 第 3 节去掉 nsh | 是 | 没有原生应用，退回手机网页 | 取决于 wasm 关卡 | 桌面看起来可用，Web 未闭合 | 放弃刚做出来的 APK |
| **C. gpui-kit + Native Shell** | 第 3 节全量 | 是 | nsh 原生 | 同 B | 同 B；手机交给系统控件 | 两套界面代码；编辑器在 nsh 里无着落；nsh 还在 0.x、有破坏性变更 |
| **D. Native Shell 全平台** | 桌面也用 nsh 的原生后端 | 是 | 有 | HTML | 交给系统控件 | 同 C 的编辑器问题，而且放弃了 GPUI 最适合编辑器的那部分 |
| **E. Tauri + Rust 写的网页前端**（Leptos / Dioxus 编译到 wasm） | 界面是 DOM，装在系统 WebView 里；手机也支持 | 代码是，运行时是 WebView | 有 | 天然有 | **由浏览器负责，风险最低** | 编辑器要从"自绘画布"改成"DOM 可编辑区"，等于换模型重写；性能与一致性受各平台 WebView 影响 |
| **F. Dioxus** | 同 E 的思路，一个框架管桌面 / 手机 / Web | 是 | 有（WebView） | 有 | 一份实测综述称可用 | 同 E；原生渲染器还不成熟 |
| **G. Slint** | 自绘，桌面 / 嵌入式 / 手机 / Web | 是 | 有 | 有 | 综述称完全可用 | 许可证是 GPLv3 或商业授权；富文本编辑能力未调研 |
| **H. Makepad / Xilem / egui / Iced** | 各自的自绘框架 | 是 | 不一 | 不一 | Makepad 组字不显示；Xilem 输入框显示不了中日韩文字 | 成熟度或输入法不达标，排除 |

怎么看这张表：

- **只想要"全栈 Rust"**：B 最直接，代价是暂时没有手机原生应用。
- **要"全平台 + 全栈 Rust + 输入法省心"**：E / F 是唯一今天就成立的组合，但它换掉的是我们投入最多的东西 ——
  自绘编辑器。这是另一个产品决策，不是换框架。
- **C 是和提出的三个概念最贴合的路线**，也是长期最有想象力的（同一家公司在同时推进桌面、Web、手机三块），
  但今天它的手机那一半没法承载编辑器。
- **A 仍然是默认选项**：我们刚用很小的代价把 Linux 和 Android 带上，Flutter 在"多一个平台"这件事上还在给回报。

## 5. 收益与风险（相对上一篇的增量）

**收益不变**：单一语言与工具链；编辑器直连 Rust 核心，不再每次操作跨 FFI / wasm 传 JSON；一批门禁可以退役。

**新增或变重的风险**：

1. **移动端从"不考虑"变成"要回答"**。上游 GPUI 没有手机支持，nsh 能不能放自绘编辑器没有证据。
   这是本次评估里最大的新增未知。
2. **刚做的手机交互要重做**。手指选择（把手、菜单、长按）和 gh#171 的修复都建立在 Flutter 的手势与隐藏输入框上。
3. **Web 输入法**：现在有了具体证据（候选窗定位的两个 PR 尚在处理），不再只是猜测。
4. **重写规模变大**：编辑器约 2.6 万行、外壳约 5.7 万行、测试 315 个文件；上一篇粗估的"全职 3–6 个月"
   只覆盖桌面 + Web，加上手机只会更长。这个数字未经拆解，仅作量级参考。
5. **三个 0.x 依赖叠在一起**（gpui-kit、gpui-fast、nsh），都由同一家公司主导，接口还在变。

## 6. 决策与进入条件

**当前决策：不迁，维持 Flutter UI + Rust 数据面。** 理由是移动端没有可验证的落地路径，Web 输入法没有闭合。

重新评估时先做 **2 周左右的限时验证**，四道关卡全过才立项；任一不过就在这里记下日期和原因。

1. **Web 关卡**：gpui-kit 编译到 wasm，做一个最小块编辑器；在 Chrome / Safari / Firefox 上实测拼音组字、
   候选窗位置、选区、复制粘贴、高分屏。
2. **桌面关卡**：Windows 与 Linux 原生窗口下的中文输入法（含是否重复上屏）、中英混排字体、托盘、富文本剪贴板。
3. **性能关卡**：2,000 块、含表格和公式的文档，对比当前 Flutter 版本
   （口径参考 `clients/mica_flutter/test/web_replica_bench_test.dart`）；同时决定要不要开 `gpui-fast`。
4. **移动关卡（新增）**：用 nsh 做一个最小 Android 应用，验证三件事 —— 能不能嵌入一块自绘区域并拿到触摸事件；
   中文输入法在其中能否组字；它和 gpui-kit 的界面代码能共用多少。**这一关不过，就只能选方案 B（手机用网页版），
   需要先确认愿不愿意放弃原生 APK。**

**触发重新评估的信号**（任一出现）：

- GPUI 上游发布正式的 Web 或移动平台；
- gpui-kit 的 Web 输入法 PR 合入，并有真实输入法会话的演示；
- nsh 发布 1.0，或文档里出现自定义绘制 / 富文本编辑的正式接口；
- Flutter 侧出现绕不过去的阻塞。

## 参考

- 上一篇：[UI 层从 Flutter 迁到 GPUI（gpui-kit）—— 调研与决策记录](ui-gpui-evaluation.md)
- [longbridge/gpui-kit](https://github.com/longbridge/gpui-kit)、[WebAssembly 指南](https://gpui-kit.com/docs/webassembly/)、
  [#3827 输入框光标移动通知平台](https://github.com/longbridge/gpui-kit/pull/3827)、
  [#3601 嵌入 Native Shell 应用](https://github.com/longbridge/gpui-kit/pull/3601)
- [longbridge/gpui-fast](https://github.com/longbridge/gpui-fast)、[#108 Web 输入法候选窗位置](https://github.com/longbridge/gpui-fast/pull/108)
- [longbridge/nsh](https://github.com/longbridge/nsh)、[nsh.dev](https://nsh.dev/)、
  [v0.3.0 发布说明](https://github.com/longbridge/nsh/releases/tag/v0.3.0)、
  [#217 GPUI 改为嵌入后端](https://github.com/longbridge/nsh/pull/217)
- [Zed 讨论 #8203：GPUI WASM](https://github.com/zed-industries/zed/discussions/8203)、
  [Zed #56327：中文输入法干扰编辑器](https://github.com/zed-industries/zed/issues/56327)
- [A 2026 Survey of Rust GUI Libraries](https://blog.wybxc.cc/blog/rust-gui-survey-2026/)（各框架输入法实测）
- [Rust GUI frameworks in 2026](https://gpui-starter.freeoxide.com/blog/rust-gui-frameworks-comparison-2026/)（平台覆盖与许可证）
