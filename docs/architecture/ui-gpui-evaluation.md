# 架构转型方向：UI 层从 Flutter 迁到 GPUI 生态，成为全平台全栈 Rust —— 调研、验证与决策记录

> **状态：候选方向，未立项。当前决策：不迁，维持 Flutter UI + Rust 数据面。**
> 跟踪 issue：[#172](https://github.com/weironz/mica/issues/172)。
>
> 本文合并了两次调研（2026-11-01 只看桌面 + Web；2026-11-10 把手机算进来）和一次部分验证（2026-11-10）。
> 重新评估时从这里开始：先复核第 2 节的事实是否仍然成立，再看第 7 节的进入条件。
>
> **一句话结论**：四道进入关卡没有一道完整通过。如果将来迁，有两条路线（第 3 节），
> 其中"全平台都用 Native Shell 自带的富文本编辑器"比"手写 GPUI 编辑器"更现实，但它自己还没有稳定。
>
> **事实来源**：第 2 节来自各项目主页、文档、源码和发布说明；第 8 节是我们在一台云端 Linux 机器上实际跑出来的，
> 范围有限。没验证的地方都标了"未验证"。

## 1. 问题与我们自己的前提

**问题**：能不能把客户端 UI 从 Flutter 迁到 GPUI 生态（Zed GPUI、gpui-kit、Native Shell），
让 Mica 成为全平台、全栈 Rust 的项目，完全删掉 Flutter？迁的话引入哪套技术栈？还有哪些别的方案？

**前提（2026-11-10 实测）**：

- **数据面已经在 Rust**：文档核心与 CRDT（mica-core `MicaDoc`）、Markdown 引擎（`crates/md-core`）、本地存储、
  导入导出、页树规则（`tree_rules`）、接口类型（`mica-api-types`）。桌面经 FFI、Web 经 wasm（`crates/md-wasm`、
  `crates/doc-wasm`）调用。**迁移只涉及 UI 层。** Rust 侧（`crates/` 与桥接层）约 10.7 万行。
- **Flutter 客户端规模**：`lib/` 约 8.3 万行 Dart（不含生成代码与 l10n），其中自绘编辑器（`lib/editor/`）约 2.6 万行；
  测试 315 个文件。
- **现役平台**：Windows 桌面、Web、Linux 桌面（.deb）、Android（APK，0.14.4 起随 Release 发布），共用一套 Dart 代码。
  手机网页版也在持续打磨（gh#171、手指选择）。所以"全平台"必须包含手机。
- 编辑器是自绘画布，IME / CJK、表格单元格、公式、Mermaid 预览、选区与撤销是历史上问题最多的区域，
  经验记录在 [开发教训](../dev/lessons.md)。

## 2. 调研对象的事实

### 2.1 Zed 的 GPUI

Zed 编辑器自带的 Rust UI 框架，GPU 渲染、自己排版。

- **桌面**：macOS、Windows、Linux 是正式平台（Zed 本身三端发布；Zed 1.0 于 2026-04 发布）。
- **Web**：上游把"Zed on the web"列在"1.0 之后"的路线里，没有正式 Web 平台。
- **移动端**：上游没有 iOS / Android 支持，只有社区项目。
- **中文输入法**：Zed 曾出现 Windows 上遇到输入法直接崩溃，后续修复；另有一条 Windows 上中文输入法重复上屏的问题单
  （#56327，原因是组字期间没有屏蔽按键消息），是否已修复未验证。
- **Zed 到了 1.0，不等于 GPUI 这个库承诺了接口稳定**；GPUI 库自身的版本与稳定性承诺未核实。

### 2.2 gpui-kit（longbridge）

基于 GPUI 的桌面应用框架，原 `gpui-component` 仓库现在就是它。约 2 万星，Apache-2.0，用于 Longbridge Pro。
2026-11-10 时 crates.io 上是 0.9.1。

- **分层**：`gpui-kit`（应用只依赖这一个 crate）、`gpui-base`（无样式的行为与基础设施）、
  `gpui-component`（带样式的组件）、`gpui-shell`（JS 扩展宿主，可选）。
- **平台**：macOS / Windows / Linux，以及浏览器（`wasm32-unknown-unknown`）。**不含移动端。**
- **和我们相关的组件**：输入框（单行、多行、自动增高）、代码编辑器（tree-sitter、LSP，自称 20 万行稳定）、
  Markdown / HTML 渲染、虚拟列表（含变高）、表格、Dock 布局、图表、AccessKit 无障碍、无头窗口 UI 测试。
  **没有富文本块编辑器**，代码编辑器是纯文本的。
- **Web 上的输入法**：用一个隐藏的 HTML 输入框接收键盘和组字事件，和 Flutter Web 的做法一样。
  有两个 PR（gpui-kit #3827、gpui-fast #108）在修"候选词窗口出现在页面角落"，我们实测该问题在 0.9.1 上存在（8.1）。
- **桌面上的输入法**：一份 2026 年的 Rust GUI 实测综述把 GPUI Component 列进了输入法可用的一档。我们没有自测。
- 每个版本固定一份上游 GPUI 快照（0.9.1 固定 `gpui-pre 0.3.12`）；破坏性变更政策未见说明。

### 2.3 gpui-fast（longbridge）

GPUI 的性能分支：保留未变化的视图、布局和文字测量，滚动复用 GPU 贴片；还能把原生内容（如 WebView）嵌进窗口。
自称"实验性"，约 500 星。在 gpui-kit 里是一个可选开关。**对我们是可选项，不是必选项。**

### 2.4 Native Shell（longbridge/nsh）

"写一次，各平台用原生控件渲染"的 UI 框架，Rust 核心，界面可以用 Rust 写，也可以用 TSX 写。约 2,300 星，Apache-2.0，0.4.0。

| 平台 | 渲染后端 |
| --- | --- |
| iOS（15+） | UIKit、CoreText |
| Android（8+） | Android 原生 View、`StaticLayout` |
| macOS / Windows / Linux | AppKit / Win32 + Direct2D / GTK 4；另有一个 GPUI 后端 |
| Web | HTML + CSS |
| 终端 | ratatui |

- 共用的是响应式引擎（Solid 风格的 signal）、flex / grid 布局、组件与状态；文字、输入、无障碍交给各平台自己的实现。
- **它和 GPUI 的关系**：GPUI 只是它的后端之一，定位是"嵌入 GPUI / gpui-kit 应用时用"，独立应用走各系统自己的后端；
  反过来 gpui-kit 可以嵌入 nsh 应用（gpui-kit #3601）。分层刻意对应 gpui-kit：`nsh-core` ≈ `gpui`，`nsh-base` ≈ `gpui-base`，
  不提供带样式的组件库。
- **和我们最相关的三样东西**（读源码确认）：
  - `Canvas` 元素：HTML Canvas 2D 的子集，Rust 侧录制绘图指令；
  - **`RichEditor`**：ProseMirror 的 Rust 移植（文档模型、步骤与事务、撤销、输入规则、表格、Markdown 读写、节点视图），
    自带触屏选择（长按选词、两端把手、iOS / Android 的系统编辑菜单）和输入法组字。
    文档自述分三期：内核已完成，各平台原生视图进行中，协同编辑列为"以后"；
  - 代码编辑器 `Editor`、只读的 Markdown / HTML 视图 `TextView`。
- **成熟度**：它的 `AGENTS.md` 写明还没有任何版本被记录为兼容基线，0.3.0 和 0.4.0 的接口都可能再变。

### 2.5 gpui-kit-markdown-editor（wabzqem，社区项目）

块式 WYSIWYG Markdown 编辑器，12 星、80 提交、无发布；只在 Linux 虚拟显示器下开发测试，
已知限制包括"空段落保存时消失"。**不能作为编辑器基础，只能参考。**

## 3. 如果切过去，引入哪一套栈

有两条路线，区别在编辑器从哪来。

### 路线一：gpui-kit（桌面 + Web）+ Native Shell（手机），编辑器手写

| 层 | 选择 | 说明 |
| --- | --- | --- |
| 桌面 UI（Windows / Linux / macOS） | gpui-kit（固定版本；`gpui-fast` 按性能实测决定） | 外壳、页树、对话框、设置用现成组件 |
| Web UI | gpui-kit 编译到 wasm | 与桌面同一套代码 |
| 手机 UI | Native Shell | 目前同一生态里唯一有正式移动后端的 |
| 编辑器 | 桌面 / Web：自己写的 GPUI 元素，直接调用 `mica-core` / `md-core`；手机：nsh 的 `RichEditor` | **两套编辑器** |

硬伤：手机和桌面不是同一个 UI 框架，外壳界面和编辑器都要写两遍。它是"全栈 Rust"，
但不是"一套界面代码跑全平台"——后者正是 Flutter 现在给我们的。

### 路线二：全平台都用 Native Shell 的 `RichEditor`

桌面走 nsh 的原生后端（或把 nsh 视图嵌进 gpui-kit 外壳），手机走原生，Web 走 HTML。一套界面代码、一个编辑器。

代价：文档模型换成 ProseMirror 式的；要自己写它和 `mica-core`（CRDT）之间的绑定（nsh 把协同列为"以后"）；
大文档性能要解决（8.2）；nsh 自己还没稳定。

### 两条路线共同要补的

| 项 | 做法 |
| --- | --- |
| 文字与输入法 | 框架自带；Web 走隐藏输入框。中日韩输入是全栈风险最高的一处 |
| 中文字体（Web） | 随应用下发，和现在一样（8.1 实测不带字体会显示成方框） |
| 公式 | 已有的 Rust `ratex-*`，不用新增 |
| 图片 / SVG / Mermaid | 框架自带图片与 SVG；Mermaid 目前靠 JS 出图，桌面端怎么跑要单独设计 |
| 托盘、文件选择、富文本剪贴板、窗口管理、自动更新 | 逐项选 Rust crate 或自己写（nsh 自带托盘和更新器，未验证）；每个新依赖按 AGENTS.md 逐项豁免 |
| 多语言 | 需要新选型（现在靠 Flutter 的 gen-l10n；nsh 有 `nsh-i18n`，未验证） |
| 测试 | 框架的无头测试 + 现有浏览器套件改造；315 个 Flutter 测试文件不能直接迁移 |

**可以删掉的**：Flutter / fvm、flutter_rust_bridge、Cargokit、生成的桥接代码、FFI 库指纹校验、
"Dart 里不许出现 Markdown 语法"一类门禁、Android 的 Gradle 工程。

## 4. 其他方案

| 方案 | 做法 | 全栈 Rust | 手机 | Web | 中文输入法 | 对我们的主要问题 |
| --- | --- | --- | --- | --- | --- | --- |
| **A. 维持现状** | Flutter UI + Rust 数据面，继续把规则下沉到 Rust | 否 | 有（已发 APK） | 有 | 已踩平大部分坑 | 两种语言、桥接层和门禁要长期维护 |
| **B. 只用 gpui-kit，手机用它的网页版** | 路线一去掉 nsh | 是 | —— | 有 | Web 候选窗位置有问题 | **已否定**：gpui-kit 的网页版在手机上不能打字（8.1） |
| **C. 路线一** | gpui-kit + Native Shell，编辑器手写 | 是 | nsh 原生 | gpui-kit wasm | 同 B；手机交给系统 | 两套界面、两套编辑器 |
| **D. 路线二** | 全平台 Native Shell + 它的 `RichEditor` | 是 | 有 | HTML | Web 实测可用（8.1）；其余交给系统，未验证 | 换文档模型、自写 CRDT 绑定、大文档性能、nsh 未稳定 |
| **E. Tauri + Rust 写的网页前端**（Leptos / Dioxus 编译到 wasm） | 界面是 DOM，装在系统 WebView 里；手机也支持 | 代码是，运行时是 WebView | 有 | 天然有 | 由浏览器负责，风险最低 | 编辑器要从"自绘画布"改成"DOM 可编辑区"，等于换模型重写；受各平台 WebView 影响 |
| **F. Dioxus** | 同 E 的思路，一个框架管桌面 / 手机 / Web | 是 | 有（WebView） | 有 | 一份实测综述称可用 | 同 E；原生渲染器还不成熟 |
| **G. Slint** | 自绘，桌面 / 嵌入式 / 手机 / Web | 是 | 有 | 有 | 综述称完全可用 | 许可证是 GPLv3 或商业授权；富文本编辑能力未调研 |
| **H. Makepad / Xilem / egui / Iced** | 各自的自绘框架 | 是 | 不一 | 不一 | Makepad 组字不显示；Xilem 输入框显示不了中日韩文字 | 成熟度或输入法不达标，排除 |

怎么看这张表：

- **A 仍然是默认选项**：我们刚用很小的代价把 Linux 和 Android 带上，Flutter 在"多一个平台"这件事上还在给回报。
- **D 是 GPUI 生态里最值得继续盯的**：一套代码、现成的富文本编辑器、同一家公司在同时推进桌面、Web、手机。
- **E / F 是今天就成立的"全平台 + 全栈 Rust + 输入法省心"**，但它换掉的是我们投入最多的自绘编辑器，
  是另一个产品决策，不是换框架。
- 只迁桌面、Web 留在 Flutter 的做法不考虑：两套 UI 代码，比现在更差。

## 5. 收益与风险

**收益**：

1. **单一语言、单一工具链**：去掉 Flutter / fvm、flutter_rust_bridge、Cargokit、生成桥接代码与 FFI 库指纹校验。
2. **编辑器直连 Rust 核心**：不再每次操作经 FFI / wasm 传 JSON；"Markdown 只有一份实现""CRDT 规则只有一份"
   从靠门禁维持变成天然成立，`check-dart-markdown.sh` 一类门禁可以退役。
3. **桌面体验潜力**：GPUI 为编辑器场景设计（Zed），gpui-kit 自带虚拟列表、代码编辑器等组件。

**风险**：

1. **中文输入法**：Web 上 gpui-kit 的候选窗位置有问题（已实测）；Windows / Linux 原生窗口有历史崩溃和重复上屏记录，未自测。
2. **移动端**：上游 GPUI 没有手机支持；nsh 的手机后端和 `RichEditor` 在真机上没有验证过。
3. **等于重写 UI**：编辑器约 2.6 万行、外壳约 5.7 万行；`lessons.md` 里的问题大概率重新出现；
   刚做的手机手势（选择把手、菜单、长按）和 gh#171 的修复都要重做；315 个测试文件不能迁移，
   门禁（浏览器套件、Playwright 截图、FFI 指纹）要按新栈重建。
4. **生态缺口需自补**：见第 3 节"共同要补的"。
5. **依赖不稳定**：gpui-kit、gpui-fast、nsh 都在 0.x，由同一家公司主导，接口还在变；nsh 明说尚未冻结任何接口。
6. **体积**：gpui-kit 的 wasm 示例 18 MB（未压缩），我们现在的两个 wasm 各约 0.25 MB。
7. **工作量**：桌面 + Web 粗估全职 3–6 个月，加上手机更长。这个数字按现有代码量估算，未经拆解，只作量级参考。

## 6. 迁完是不是纯 Rust？Flutter 能不能完全删？

能，但前提是 Web 和手机都迁过去。Web 仍会有少量 JS 加载胶水（`index.html`、wasm-bindgen 生成代码），不算第二套实现。
**能否删干净，取决于新栈在浏览器和手机上能否撑起编辑器，尤其是中文输入。**

## 7. 决策与进入条件

**当前决策：不迁。** 理由：四道关卡没有一道完整通过；Web 输入法有实测问题；手机端没有真机证据。
近期的 Rust 化（md-wasm、doc-wasm、`mica-api-types`、`tree_rules`）无论将来迁不迁都是可复用资产。

重新评估时做**限时验证**，四道关卡全过才立项；任一不过就在第 8 节记下日期和原因。

1. **Web 关卡**：在 Chrome / Safari / Firefox 上实测拼音组字、候选窗位置、选区、复制粘贴、高分屏。
2. **桌面关卡**：Windows 与 Linux 原生窗口下的中文输入法（含是否重复上屏）、中英混排字体、托盘、富文本剪贴板。
3. **性能关卡**：2,000 块、含表格和公式的文档，对比当前 Flutter 版本
   （口径参考 `clients/mica_flutter/test/web_replica_bench_test.dart`）。
4. **移动关卡**：Android 真机上的组字、长按选词、把手、系统菜单；以及和桌面能共用多少界面代码。

**触发重新评估的信号**（任一出现）：

- GPUI 上游发布正式的 Web 或移动平台；
- gpui-kit 的 Web 输入法修复合入，并有真实输入法会话的演示；
- Native Shell 记录了第一个兼容基线版本，或 `RichEditor` 的各平台视图标为完成；
- Flutter 侧出现绕不过去的阻塞（如 Web 编辑器输入法问题长期无解）。

## 8. 关卡验证记录（2026-11-10，云端 Linux 机器）

**这次只做了能在一台无显示器、无硬件虚拟化的 Linux 机器上做的部分**：只有 Chromium，没有 Safari / Firefox，
没有 Windows，没有 Android 模拟器。输入法用浏览器调试协议模拟组字（发送"n → ni → ni h → ni hao"再提交"你好"），
不是真人打字。所以下面的"通过"只代表 Chromium 里的这条路径通了。

| 关卡 | 结果 | 做了什么 |
| --- | --- | --- |
| 1. Web | **未通过**（Chromium 部分做完，发现三个问题；Safari / Firefox 未测） | 见 8.1 |
| 2. 桌面 | **未执行** | 需要 Windows 真机和真人用输入法打字；Linux 原生窗口的输入法也没有试 |
| 3. 性能 | **只有 Web 上的粗测**；Native Shell 的示例未达标，GPUI 未测 | 见 8.2 |
| 4. 移动 | **未执行运行验证**，只读了源码 | 见 8.3 |

### 8.1 Web 关卡

**gpui-kit 0.9.1**（本地编译官方 `gpui-base` 的 wasm 示例，测其中的多行输入框）：

- 能编译：需要固定的 nightly 工具链（`nightly-2026-10-31`）和 wasm-bindgen 0.2.127；release 构建 2 分 45 秒（4 核），
  产物 18 MB（未压缩）。
- 通过：英文输入、组字提交（"你好""世界"连续两次）、取消组字不留残字、撤销、全选复制（剪贴板内容正确）。
- **问题 1：候选词窗口位置。** 组字期间，接收输入的隐藏文本框一直停在页面左上角 `(0, 0)`，没有跟随光标 ——
  真实输入法的候选窗会出现在页面角落。和 gpui-kit #3827 / gpui-fast #108 描述的是同一个问题。
  （想用 `gpui-fast` 开关重测，示例在该开关下编译失败，未测成。）
- **问题 2：中文显示为方框。** 浏览器里拿不到系统字体，示例没带中文字体。可以解决（随应用下发字体），但必须自己补。
- **问题 3（2026-11-10 那次构建/Chromium 快照）：手机上不能打字。** 示例自己的源码注释写着：通过屏幕外的输入框往画布里打字，在手机上不可用，
  所以触屏设备上它把输入框设成只读、不弹键盘。**这是当次否定方案 B 的依据；当前源码/DOM差异见8.5，不能将其扩大为所有新构建永远不能移动输入。**
- 另：无头 Chromium 的 WebGPU 路径一启动就丢失设备、画面空白，退回 WebGL 才正常。多半是本机软件渲染的问题，只作记录。

**Native Shell 的 `RichEditor`**（官网在线示例，浏览器里是 HTML 渲染）：

- 通过：英文输入、Markdown 快捷输入（`## ` 变标题）、组字期间文档不变、提交、连续组字、取消、撤销、全选复制。
- **隐藏输入框跟随光标**（组字时位于光标处），候选窗位置应当正确。
- 手机模拟（Pixel 7 + 触摸事件）：`main` 分支的构建上长按能选中一个词；已发布版本上长按没有反应。
  两端把手没有出现，三击没有选中整段。触屏这部分还在开发中。

### 8.2 性能关卡（只有 Web，文档不含表格和公式）

同一份 2,000 块文档（标题、中英混排段落、列表、待办），同一台机器、同一个无头 Chromium：

| | Mica 现在的 Flutter 编辑器 | Native Shell `RichEditor` 在线示例 |
| --- | --- | --- |
| 每次按键到画面更新（20 次的中位数） | **33 ms** | **283 ms** |
| 滚动时的帧间隔（95 分位） | 17 ms | 17 ms |
| 载入 | 页面加载后直接显示 | 粘贴进去用了 13.4 秒 |
| 页面元素数 | 画布，不产生元素 | 约 19,900 个（整篇都渲染，没有只渲染可见部分） |

- Native Shell 的数字**偏悲观**：那个示例页每次改动都会把整篇文档重新导出成 Markdown 显示在旁边，这部分开销算在了里面。
  但"整篇渲染、不做可见区域裁剪"是实打实的。
- **GPUI 没有测**：测它需要先写一个块编辑器原型，这次没有做。gpui-kit 现成的虚拟列表和我们的编辑器不可比。
- 原定的"含表格和公式"没有覆盖。

### 8.3 移动关卡

- **没有做运行验证**：这台机器跑不了 Android 模拟器，也没有真机。
- 读源码确认：Native Shell 有 `Canvas`、有 `RichEditor`，Android / iOS 后端里有系统编辑菜单和输入法的接入代码（见 2.4）。

### 8.4 这次验证改变了什么

1. **当次方案 B 被否定**：当次 gpui-kit Web 手机样本不能打字；后续构建差异不等于移动关卡已通过。
2. **路线二成立为候选**：验证前只设想了"手写 GPUI 编辑器"，读源码后发现 Native Shell 有现成的富文本编辑器。
3. **结论不变：现在不迁。**

**还需要有人做的**（按价值排序）：

- 在 Windows 上用真实的中文输入法试 gpui-kit 原生窗口和 Native Shell 的 Windows 后端（关卡 2）；
- 在 Android 真机上跑 Native Shell 的 `RichEditor` 示例：组字、长按、把手、系统菜单（关卡 4）；
- Safari 和 Firefox 上重复 8.1；
- 如果倾向路线二，做一个"`RichEditor` + `mica-core`"的最小绑定，再测 2,000 块含表格和公式的文档。

### 8.5 2026-11-11 Firefox复验与边界

官方gpui-kit v0.9.1/39ab849原源码与锁，隔离nightly-2026-10-31+bindgen0.2.127重建1m39s（wasm18,077,402 B）。官方Firefox159.0.1/driver0.37.1在Xvfb+llvmpipe、禁WebGPU的WebGL2基线中，真实键盘英文/全选复制及合成composition连续提交/取消/撤销通过；每步由真实GPUI复制回读确认。汉字仍方框；组字textarea镜像固定(0,0,8,8)，未跟随绘制光标。没有真人系统候选窗验证。

当前输入镜像是TEXTAREA、readOnly=false。示例手机抑制脚本只处理INPUT，平台ImeMirror另有touch_input/inputmode切换；因此旧8.1“手机不能打字”仅代表原快照，不能用那段源码注释概括本次构建，移动真实交互仍待设备。

NativeShell当日RichEditor在同版Firefox于加载前报nsh:rt动态模块错误，而Chromium152同URL正常。独立最小诊断证明Firefox必须先注册import map再加载模块；当前官网runtime先加载bootstrap模块、随后追加map，触发兼容失败。未加载不能算输入/撤销/剪贴板关卡通过。Safari、Windows真人中文输入、Android真机本轮仍未完成，不用WebKit/模拟触摸/非trusted事件替代。

四道关卡仍未完整通过，维持“不迁UI”的决策。证据与具体限制见[第三轮实施记录](../reviews/redundancy-and-home-review-2026-11-10.md#96-gpui-剩余关卡)。

## 9. 调研时间线

| 日期 | 做了什么 | 结论 |
| --- | --- | --- |
| 2026-11-01 | 第一次调研：gpui-kit、社区 Markdown 编辑器；只看桌面 + Web | 不迁；定了 Web / Windows / 性能三道关卡 |
| 2026-11-10 | 第二次调研：加入 Native Shell、gpui-fast 和其他 Rust UI 方案；Linux 与 Android 已成为现役平台 | 不迁；新增移动关卡 |
| 2026-11-10 | 部分验证（第 8 节）；读 Native Shell 源码，更正了"它没有自绘接口和富文本编辑器"的错误判断 | 不迁；方案 B 否定，路线二成为候选 |

## 参考

- [longbridge/gpui-kit](https://github.com/longbridge/gpui-kit)、[WebAssembly 指南](https://gpui-kit.com/docs/webassembly/)、
  [#3827 输入框光标移动通知平台](https://github.com/longbridge/gpui-kit/pull/3827)、
  [#3601 嵌入 Native Shell 应用](https://github.com/longbridge/gpui-kit/pull/3601)
- [longbridge/gpui-fast](https://github.com/longbridge/gpui-fast)、[#108 Web 输入法候选窗位置](https://github.com/longbridge/gpui-fast/pull/108)
- [longbridge/nsh](https://github.com/longbridge/nsh)、[nsh.dev](https://nsh.dev/)、
  [RichEditor 在线示例](https://nsh.dev/elements/rich-editor)、
  [v0.3.0 发布说明](https://github.com/longbridge/nsh/releases/tag/v0.3.0)、
  [#217 GPUI 改为嵌入后端](https://github.com/longbridge/nsh/pull/217)
- [wabzqem/gpui-kit-markdown-editor](https://github.com/wabzqem/gpui-kit-markdown-editor/)
- [Zed 讨论 #8203：GPUI WASM](https://github.com/zed-industries/zed/discussions/8203)、
  [Zed #56327：中文输入法干扰编辑器](https://github.com/zed-industries/zed/issues/56327)、
  [Zed #9207：Linux 死键 / IME](https://github.com/zed-industries/zed/issues/9207)、
  [GPUI Web 社区分支（hugomoran159/zed）](https://github.com/hugomoran159/zed)
- [A 2026 Survey of Rust GUI Libraries](https://blog.wybxc.cc/blog/rust-gui-survey-2026/)（各框架输入法实测）、
  [Rust GUI frameworks in 2026](https://gpui-starter.freeoxide.com/blog/rust-gui-frameworks-comparison-2026/)（平台覆盖与许可证）
- 本仓库：[单一实现专项](../single-source/README.md)、[Web CRDT 迁移记录](../reviews/crdt-web-wasm-spike-2026-11-01.md)、
  [开发教训](../dev/lessons.md)
