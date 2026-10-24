# 2026-10-24 列表跨块格式与表格编辑边框修复

基于 `cca4b16`（包括近期已关闭 issue 的修复）继续处理用户反馈，不回退整批变更，
不修改 GitHub issue 状态，不部署生产或修改其他项目。

## 根因与行为

- 无序列表每项是一个块。格式命令遇到 `isMultiNode` 就返回，浮动工具条也隐藏跨块
  选区的行内格式。现在按选区与各文本块的交集处理首尾字符和中间文本，跳过代码块、
  表格、图片与空范围。支持加粗、斜体、删除线、行内代码和链接；全部已有格式时统一
  移除，否则统一添加。保留块类型、缩进、待办属性、正文及选区方向。
- 一次格式操作仅发一个 op batch，作为一个撤销/重做步骤。范围逻辑放进独立库
  `selection_marks.dart`，不靠 part 绕过库行数门禁。
- 表格单元格仅设置 `border: InputBorder.none` 仍会继承主题的 `enabledBorder` /
  `focusedBorder`。现在取消各状态的输入框边框及填充；renderer 不再叠加整表选择
  描边，编辑单元格时也不盖整表选择色。保留网格、表头、行列范围选择和列宽调整。
- 链接补测另复现了对话框返回时提前 dispose 输入控制器。使用 `DialogRoute`，退出
  完成后释放控制器，保留根 Navigator 与主题捕获。
- Web 实编译发现 `package:web` external extension member 不允许直接 tear-off：
  `iframe.remove` 改为 `() => iframe.remove()`，保持延迟移除打印 iframe 的原行为。

## 与已关闭 issue 的关系

| Issue | 本轮保留并回归的边界 |
| --- | --- |
| [#51](https://github.com/weironz/mica/issues/51) | 远端更新后的选区映射，不还原旧的折叠选区逻辑 |
| [#55](https://github.com/weironz/mica/issues/55) | AI 采用更新后的选区，跨块格式不隐藏 AI 按钮 |
| [#79](https://github.com/weironz/mica/issues/79) | 本地逆操作历史；批量格式复用现有提交/历史通道 |
| [#82](https://github.com/weironz/mica/issues/82) | 表格命中与选择绘制仍在 renderer 注册表，不把分支移回宿主 |
| [#87](https://github.com/weironz/mica/issues/87) | 按库统计；本次三个存量编辑器库均在原预算内，不提高预算 |
| [#89](https://github.com/weironz/mica/issues/89) | 保留 package:web 迁移，仅修正编译不接受的回调形式 |

## 验证

- 新增控制器用例覆盖五类标记、混合格式、反向选区、首尾边界、范围外格式、跳过
  不可格式化块、批量提交、一次撤销/重做与无效操作不提交。
- 新增组件用例覆盖浮动条、快捷键、外壳 hook、跨列表链接对话框、深浅色输入框、
  真实栅格像素（无额外描边而保留网格）及单元格保存。更新旧“跨块隐藏加粗”断言，
  保留 AI 用例。失败回归在修复前已实测。
- 25 个相关测试文件共 **193 项通过**，涵盖选区 reconcile、历史、AI、复制/粘贴标记、
  表格拖动、范围操作、滚动跟随与编辑控制器生命周期。
- 本地 Chrome 使用真实 MicaEditor 的内存文档测试页：拖选三条列表、点击格式条后
  三条数据均含 `bold,italic`；深浅色单元格输入后 Esc 提交保留新文字，截图无额外框。
  未连接生产、使用凭据或更改用户文档；未将测试页作为完整生产客户端验收。
- Flutter Web 编译及 hot reload 成功，运行时错误查询为空。截图与浏览器收据位于
  `%LOCALAPPDATA%/Temp/mica-editor-regression-20261024`；日志位于本地
  `target/editor-regression-20261024/flutter-tests.log`，不提交临时页与缓存。
- `flutter analyze --no-fatal-infos` 无本轮新增错误或警告，存量代码仍有既有 info；
  `git diff --check` 通过。

## 门禁及交付边界

全仓 `just check-structure` 仍失败：**基线 HEAD cca4b16 的 main.dart 库已经 21371 行，
预算 21355，超 16 行**。以 `git show HEAD:<file>` 汇总该库 18 个文件复核，且这些
文件本轮均未修改。没有放宽预算或为过闸删除无关代码，不能宣称全仓门禁通过。
本次 controller / editor / render 三库均在各自原预算内。

按协作规则尝试的 DeepSeek、GLM、Kimi 均因当前会话后端的任务封装限制被拒绝，
未参与审查；结论来自主模型的代码核对、真实测试、浏览器验证与运行时检查。

本轮仅源码修复，未 bump 版本、未发版；用户的安装包/云端需后续发布部署才获得改动。
