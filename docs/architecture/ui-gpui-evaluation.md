# 架构转型方向：UI 层从 Flutter 迁到 GPUI（gpui-kit）—— 调研与决策记录

> 2026-11-01 调研。**状态：候选方向，未立项。** 结论是"现在不迁"，先做限时验证（下文"进入条件"），
> 三道关卡都通过才按"桌面 + Web 一起迁"立项。本文是后期重新评估时的起点：先复核文中的事实是否仍然成立，
> 再看进入条件。
>
> **续篇（2026-11-10）**：[全平台全栈 Rust 评估](ui-rust-stack-evaluation-2026-11-10.md) 把移动端和 Native Shell 算了进来，
> 并新增第四道"移动关卡"；进入条件以续篇为准。

## 1. 问题

能不能把客户端从 Flutter 迁到 [gpui-kit](https://github.com/longbridge/gpui-kit)（基于 Zed 的 GPUI），
让 Mica 变成纯 Rust 项目、完全删掉 Flutter？有什么收益和风险？参考对象还有
[gpui-kit-markdown-editor](https://github.com/wabzqem/gpui-kit-markdown-editor/)。

## 2. 我们自己的前提（2026-11-01 实测）

- **数据面已经在 Rust**：文档核心与 CRDT（mica-core `MicaDoc`）、Markdown 引擎（`crates/md-core`）、本地存储、
  导入导出、页树规则（`tree_rules`）、接口类型（`mica-api-types`）。桌面经 FFI、Web 经 wasm（`crates/md-wasm`、
  `crates/doc-wasm`）调用。**迁移只涉及 UI 层。**
- **Flutter 客户端规模**：`lib/` 约 7.2 万行 Dart（不含生成代码与 l10n），其中自绘编辑器（`lib/editor/`）约 2.5 万行；
  测试 288 个文件约 4.5 万行。
- **平台**：Windows 桌面 + Web 是现役平台，两者共用这套 Dart 代码（约 50 处 Web 条件分支）。
- 编辑器是自绘画布，IME / CJK、表格单元格、公式、Mermaid 预览、选区与撤销是历史上问题最多的区域，
  经验记录在 [开发教训](../dev/lessons.md)。

## 3. 调研对象的事实（2026-11-01）

**gpui-kit**（longbridge）
- 约 1.9 万星，Apache-2.0，crates.io 发布，示例用 `gpui-kit = "0.7"`；自称 75+ 组件，用于 Longbridge Pro。
- 分层：`gpui-component`（有样式组件）、`gpui-base`（无样式行为与基础设施）、`gpui-shell`（JS 扩展）。
- 平台：项目页列出 macOS / Windows / Linux，以及 `wasm32-unknown-unknown`；**未提移动端**（另有 Native Shell 项目做 iOS/Android）。
- 组件：虚拟列表（含变高）、代码编辑器（tree-sitter、LSP，自称 20 万行稳定）、Markdown/HTML 渲染、表格、Dock 布局、图表、AccessKit 无障碍。
- **项目页未提 IME 支持。**
- 依赖"固定的上游 GPUI 快照"；GPUI 尚未 1.0。

**gpui-kit-markdown-editor**（wabzqem）
- 块式 WYSIWYG Markdown 编辑器，12 星、80 提交、无发布；起源于一次 spike。
- 只在 Linux 虚拟显示器下开发测试；macOS / Windows 的键位与 IME 已实现但未验证。
- 已知限制包括"空段落保存时消失"、无障碍基本未验证、缺 input-rule 注册表。
- **结论：不能作为编辑器基础，只能参考。**

**GPUI 的 Web 支持**
- 官方没有发布 Web 平台；查到一个社区分支自称通过 wasm + WebGPU/WebGL2 跑通全部示例（未合入上游，未验证）。
- gpui-kit 声称支持 wasm，**本次未实测**。

**GPUI 的 IME / CJK**
- Zed 曾出现 Windows 遇到 IME 输入直接崩溃，后续修复（"windows: Better keyboard input support"）；Linux 死键/IME 曾依赖 Windows IME 工作推进。
- 未找到"Windows/Linux 中文 IME 现已可靠"的明确证据，需要自测。

## 4. 收益

1. **单一语言、单一工具链**：去掉 Flutter/fvm、flutter_rust_bridge、Cargokit、生成桥接代码与 FFI 库指纹校验。
2. **编辑器直连 Rust 核心**：不再每次操作经 FFI/wasm 传 JSON；"Markdown 只有一份实现""CRDT 规则只有一份"
   从靠门禁维持变成天然成立，`check-dart-markdown.sh` 一类门禁可以退役。
3. **桌面体验潜力**：GPUI 为编辑器场景设计（Zed），gpui-kit 自带虚拟列表、代码编辑器等组件。

## 5. 风险

1. **Web 端（最大未知）**：上游无官方 Web 支持；gpui-kit 的 wasm 声明未验证。关键是 Safari / Firefox 的可用性，
   以及**画布上的中文输入法组字**——Flutter Web 靠隐藏 textarea 解决的正是这个，也是我们编辑器踩坑最多的地方。
2. **Windows / Linux 中文 IME 成熟度**：有历史崩溃记录，需实测。
3. **等于重写 UI**：2.5 万行编辑器重写，`lessons.md` 里的问题大概率重新出现；4.5 万行 Flutter 测试不能迁移，
   门禁（浏览器套件、Playwright 截图、FFI 指纹）要按新栈重建。
4. **生态缺口需自补**：托盘、富文本剪贴板、文件选择、窗口管理、自动更新、SVG 渲染（Mermaid 输出 SVG）；
   公式可复用已有的 Rust `ratex-*`。按 AGENTS.md，新增依赖都要逐项豁免。
5. **API 不稳定**：GPUI 未到 1.0，需跟随 gpui-kit 固定的快照升级。
6. **工作量**：粗估全职 3–6 个月（按现有代码量估算，未经拆解，只作量级参考）。

## 6. 迁完是不是纯 Rust？Flutter 能不能完全删？

| 方案 | 结果 |
| --- | --- |
| 只迁桌面 | 桌面纯 Rust，但 Web 仍是 Flutter → **两套 UI 代码，比现在更差，不采纳** |
| 桌面 + Web 都迁到 GPUI（wasm） | Flutter 可完全删除，成为纯 Rust 项目；Web 仍有少量 JS 加载胶水（`index.html`、wasm-bindgen 生成代码），不算第二套实现 |

**能否删干净，完全取决于 GPUI 在浏览器里能否撑起我们的编辑器（尤其中文输入）。**

## 7. 决策与进入条件

**当前决策：不迁，维持 Flutter UI + Rust 数据面。** 近期的 Rust 化（md-wasm、doc-wasm、`mica-api-types`、`tree_rules`）
无论将来迁不迁都是可复用资产。

重新评估时，先做 **1–2 周限时验证**，三道关卡全部通过才立项：

1. **Web 关卡**：gpui-kit 编译到 wasm，做最小块编辑器原型；在 Chrome / Safari / Firefox 实测中文拼音组字、选区、
   复制粘贴、高分屏。
2. **Windows 关卡**：原生窗口下中文 IME、中英混排字体、托盘、剪贴板。
3. **性能关卡**：2,000 块含表格和公式的文档，与当前 Flutter 版本对比（基准参考
   `clients/mica_flutter/test/web_replica_bench_test.dart` 的口径）。

任一关卡不过，维持现状并在此记录日期与原因。

**触发重新评估的信号**（任一出现即可）：GPUI 官方发布 Web 平台或到 1.0；gpui-kit 明确声明并演示 CJK IME；
Flutter 侧出现无法绕过的阻塞（如 Web 编辑器输入法问题长期无解）。

## 参考

- [longbridge/gpui-kit](https://github.com/longbridge/gpui-kit)
- [wabzqem/gpui-kit-markdown-editor](https://github.com/wabzqem/gpui-kit-markdown-editor/)
- [GPUI Web 社区分支（hugomoran159/zed）](https://github.com/hugomoran159/zed)
- [Zed issue #9207（Linux 死键 / IME）](https://github.com/zed-industries/zed/issues/9207)
- [WebGPU 主流浏览器支持（web.dev）](https://web.dev/blog/webgpu-supported-major-browsers)
- 本仓库：[单一实现专项](../single-source/README.md)、[Web CRDT 迁移记录](../reviews/crdt-web-wasm-spike-2026-11-01.md)、
  [开发教训](../dev/lessons.md)
