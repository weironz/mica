# 全平台全栈 Rust：UI 层迁到 GPUI 的可行性与技术栈选择（2026-11-10 调研）

> **状态：候选方向，未立项。** 本文是 [2026-11-01 的 GPUI 评估](ui-gpui-evaluation.md) 的续篇：那一篇只回答
> "桌面 + Web 能不能迁"，这一篇把**移动端**算进来，回答三个问题：
> 1. 现在迁过去可不可行；
> 2. 如果迁，要引入哪一套技术栈；
> 3. 除了 GPUI 还有哪些方案。
>
> **结论先行**：现在仍然不迁。如果将来迁，栈是 **gpui-kit（桌面 + Web）+ Native Shell（移动端）**，
> 两者同属 Longbridge、共用一套 Rust 组件思路。立项前先过第 6 节的四道关卡。
> 2026-11-10 的部分验证（第 7 节）否定了"手机用 GPUI 网页版"，并指出另一条路线：全平台都用 Native Shell 自带的富文本编辑器。
>
> 第 1–6 节的外部事实来自各项目主页、发布说明和搜索结果；第 7 节是 2026-11-10 当天在一台云端 Linux 机器上实际跑出来的
> 部分验证，范围有限，**四道关卡没有一道完整通过**。

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
- **对我们最关键的一点（2026-11-10 读源码后更正）**：初稿说"没有自绘接口、没提富文本编辑"，**这是错的**，
  只看了主页。仓库里有：
  - `Canvas` 元素（HTML Canvas 2D 的子集，Rust 侧录制绘图指令）；
  - **`RichEditor`**：ProseMirror 的 Rust 移植（文档模型、步骤与事务、撤销、输入规则、表格、Markdown 读写、
    节点视图），自带触屏选择（长按选词、两端把手、iOS / Android 的系统编辑菜单）和输入法组字。
    文档自述分三期：内核已完成，各平台原生视图进行中，协同编辑列为"以后"。
  - 另有代码编辑器 `Editor`、只读的 Markdown / HTML 视图 `TextView`。

  也就是说，**手机端不必把我们的自绘编辑器搬进去，可以直接用它的 `RichEditor`**；代价是文档模型换成
  ProseMirror 式的，要自己写它和 `mica-core`（CRDT）之间的绑定。实测见第 7 节。

## 3. 如果切过去，引入哪一套栈

| 层 | 选择 | 说明 |
| --- | --- | --- |
| 桌面 UI（Windows / Linux / macOS） | **gpui-kit**（固定版本；`gpui-fast` 开关按性能实测决定） | 外壳、页树、对话框、设置直接用现成组件 |
| Web UI | **gpui-kit 编译到 wasm** | 与桌面同一套代码；能否成立取决于第 6 节的 Web 关卡 |
| 手机 UI（Android / iOS） | **Native Shell**，Rust 组件 | 目前唯一同一生态、有正式移动后端的选择；编辑器可用它自带的 `RichEditor`（见 2.4、第 7 节），真机上未验证 |
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
**不是同一份界面代码**。外壳界面大概率要写两遍；编辑器在手机上可以用 nsh 的 `RichEditor`，但那和桌面上手写的 GPUI 编辑器又是两套（7.4 给出了只用一套的路线）。
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

1. **移动端从"不考虑"变成"要回答"**。上游 GPUI 没有手机支持，nsh 有自己的富文本编辑器（见 2.4 的更正），但在真机上没有验证过。
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
   中文输入法在其中能否组字；它和 gpui-kit 的界面代码能共用多少。**方案 B（手机用 GPUI 网页版）已在 7.1 被否定，所以这一关不过就没有手机端可言。**

**触发重新评估的信号**（任一出现）：

- GPUI 上游发布正式的 Web 或移动平台；
- gpui-kit 的 Web 输入法 PR 合入，并有真实输入法会话的演示；
- nsh 发布 1.0，或文档里出现自定义绘制 / 富文本编辑的正式接口；
- Flutter 侧出现绕不过去的阻塞。

## 7. 关卡验证记录（2026-11-10，云端 Linux 机器）

**这次只做了能在一台无显示器、无硬件虚拟化的 Linux 机器上做的部分**：只有 Chromium，没有 Safari / Firefox，
没有 Windows，没有 Android 模拟器。输入法用浏览器调试协议模拟组字（发送"n → ni → ni h → ni hao"再提交"你好"），
不是真人打字。所以下面的"通过"只代表 Chromium 里的这条路径通了。

| 关卡 | 结果 | 做了什么 |
| --- | --- | --- |
| 1. Web | **未通过**（Chromium 部分做完，发现三个问题；Safari / Firefox 未测） | 见 7.1 |
| 2. 桌面 | **未执行** | 需要 Windows 真机和真人用输入法打字；Linux 原生窗口的输入法也没有试 |
| 3. 性能 | **只有 Web 上的粗测**；Native Shell 的示例未达标，GPUI 未测 | 见 7.2 |
| 4. 移动 | **未执行运行验证**；读源码推翻了初稿的一个判断 | 见 7.3 |

### 7.1 Web 关卡

**gpui-kit 0.9.1**（本地编译官方 `gpui-base` 的 wasm 示例，测其中的多行输入框）：

- 能编译：需要固定的 nightly 工具链（`nightly-2026-10-31`）和 wasm-bindgen 0.2.127；release 构建 2 分 45 秒（4 核），
  产物 **18 MB**（未压缩的 wasm，比我们现在的两个 wasm 各约 0.25 MB 大两个数量级）。
- 通过：英文输入、组字提交（"你好""世界"连续两次）、取消组字不留残字、撤销、全选复制（剪贴板内容正确）。
- **问题 1：候选词窗口位置。** 组字期间，接收输入的隐藏文本框一直停在页面左上角 `(0, 0)`，没有跟随光标 ——
  真实输入法的候选窗会出现在页面角落。这和 gpui-kit #3827 / gpui-fast #108 描述的是同一个问题，在 0.9.1 默认后端上仍然存在。
  （想用 `gpui-fast` 开关重测，示例在该开关下编译失败，未测成。）
- **问题 2：中文显示为方框。** 浏览器里拿不到系统字体，示例没带中文字体，"你好世界"画成了四个方框。
  可以解决（像现在一样随应用下发中文字体），但这是必须自己补的一项。
- **问题 3：手机上不能打字。** gpui-kit 示例自己的源码注释写着：通过屏幕外的输入框往画布里打字，在手机上不可用，
  所以触屏设备上它把输入框设成只读、不弹键盘。**这直接否定了"方案 B：手机先用 GPUI 的网页版"。**
- 另：无头 Chromium 的 WebGPU 路径一启动就丢失设备、画面空白，退回 WebGL 才正常。这多半是本机软件渲染的问题，
  只作记录。

**Native Shell 的 `RichEditor`**（官网在线示例，浏览器里是 HTML 渲染）：

- 通过：英文输入、Markdown 快捷输入（`## ` 变标题）、组字期间文档不变、提交、连续组字、取消、撤销、全选复制。
- **隐藏输入框跟随光标**（组字时位于光标处），候选窗位置应当正确。
- 手机模拟（Pixel 7 + 触摸事件）：`main` 分支的构建上长按能选中一个词；已发布版本上长按没有反应。
  两端把手没有出现，三击没有选中整段。说明触屏这部分还在开发中。

### 7.2 性能关卡（只有 Web，文档不含表格和公式）

同一份 2,000 块文档（标题、中英混排段落、列表、待办），同一台机器、同一个无头 Chromium：

| | Mica 现在的 Flutter 编辑器 | Native Shell `RichEditor` 在线示例 |
| --- | --- | --- |
| 每次按键到画面更新（20 次的中位数） | **33 ms** | **283 ms** |
| 滚动时的帧间隔（95 分位） | 17 ms | 17 ms |
| 载入 | 页面加载后直接显示 | 粘贴进去用了 13.4 秒 |
| 页面元素数 | 画布，不产生元素 | 约 19,900 个（整篇都渲染，没有只渲染可见部分） |

- Native Shell 的数字**偏悲观**：那个示例页每次改动都会把整篇文档重新导出成 Markdown 显示在旁边，这部分开销算在了里面。
  但"整篇渲染、不做可见区域裁剪"是实打实的，大文档下需要它或我们自己解决。
- **GPUI 没有测**：测它需要先写一个块编辑器原型，这次没有做。gpui-kit 现成的虚拟列表和我们的编辑器不可比。
- 原定的"含表格和公式"也没有覆盖。

### 7.3 移动关卡

- **没有做运行验证**：这台机器跑不了 Android 模拟器，也没有真机。
- 读源码得到的结论（已更正到 2.4 节）：Native Shell 有 `Canvas`、有 `RichEditor`，Android / iOS 后端里有系统编辑菜单和
  输入法的接入代码。原定要验证的"能不能嵌入一块自绘区域"在接口层面是有的。
- 它的 `AGENTS.md` 写明：**还没有任何版本被记录为兼容基线，0.3.0 和 0.4.0 的接口都可能再变**；`RichEditor` 的各平台视图
  是进行中的第二期，协同编辑排在其后。

### 7.4 这次验证改变了什么

1. **方案 B 出局**：gpui-kit 的网页版在手机上不能打字，这是它自己写明的。
2. **多了一条更现实的路线**：不必手写 GPUI 编辑器，改用 Native Shell 的 `RichEditor` 覆盖全平台
   （桌面走它的原生后端或嵌进 gpui-kit，手机走原生，Web 走 HTML）。这样"手机和桌面是两套界面代码"的硬伤可以消掉。
   要付的代价：文档模型换成 ProseMirror 式；要写它和 `mica-core` 的绑定；大文档性能要解决；它自己还没稳定。
3. **结论不变：现在不迁。** 四道关卡里没有一道完整通过，其中两道还没有条件执行。

**还需要有人做的**（按价值排序）：

- 在 Windows 上用真实的中文输入法试 gpui-kit 原生窗口和 Native Shell 的 Windows 后端（关卡 2）；
- 在 Android 真机上跑 Native Shell 的 `RichEditor` 示例：组字、长按、把手、系统菜单（关卡 4）；
- Safari 和 Firefox 上重复 7.1；
- 如果倾向 7.4 的第 2 条路线，做一个"`RichEditor` + `mica-core`"的最小绑定，再测 2,000 块含表格和公式的文档。

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
