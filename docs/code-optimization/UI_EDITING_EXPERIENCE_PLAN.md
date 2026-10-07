# UI 与编辑舒适感优化方案

日期：2026-10-07。基线：`0aa1266`，生产界面 v0.13.47。

## 目标与证据

让 Mica 的中文长文和技术文档更易读，让输入、光标、选区与滚动反馈更稳定，并将设置页整理为完整的产品界面。本方案先落盘，再按下表开发与验证；不等待逐项批准。

已检查生产浅色桌面、390px 宽视口、编辑器与设置源码：

- 页面标题用 Material headlineMedium（28px、常规字重），正文 H1 更大、更粗；长页面标题是单行输入框。
- 当前用户偏好为正文17px、页宽1040px；默认页宽800px。优化保留已有偏好，不偷偷覆盖它们。
- 设置内容固定720×460、左栏180px，窄屏下部分控件与关闭按钮不可见。
- 控件继承 Material 默认形状与次强调色，与自绘导航、工具栏不协调。
- `setSelection` 重复通知；选区通知触发全篇布局、图片/目录扫描和字数定时器。光标闪烁本身已经是 paint-only，不应重复改造。
- 拖选未结束就出现浮动格式栏；手势取消缺少清理入口；边缘自动滚动由16ms Timer驱动。
- 公式字体被重复缩放；`EditorAppearance` 相等判定未包含调色板，主题切换可能保留旧画布颜色。

“顺滑”不能用加动画代替：正文输入与光标定位即时更新，选区不做位移动画，合成输入的文本与提交语义保持原样。

## 设计规则

方向：安静、清晰，适合反复阅读和编辑中文长文、代码与公式。

- 保留正文16px基准乘已有字号偏好、1.65行高；协调页面标题与正文标题，标题不使用负字距。
- 页面标题使用独立样式（桌面约34px/600，窄屏约30px/600），允许自然换行；Enter继续进入正文，IME候选确认不得误触发。
- 统一段落、列表、章节的间距；标题上方大于下方，保持与后续正文的归属感。
- 白色正文、轻微中性灰外壳；强调色用于操作与选中态。统一菜单、按钮、输入框的形状、密度与状态，不增加装饰性卡片或渐变。
- 设置页宽屏双栏、窄屏分类列表进入详情；返回和关闭常驻，内容独立纵向滚动。
- 外观预览包含中英混排、标题、正文、代码；展示当前参数，调整即时生效。
- 不新增依赖、不更换编辑器、CRDT和Markdown模型；保留全部设置功能与回调。

## 实施清单

本方案先于源码修改落盘。下表更新为实施状态；详细证据、回归有效性和平台限制见 [验证记录](UI_EDITING_EXPERIENCE_VALIDATION.md)。位置为基线位置，实施后以符号名定位。

| ID / 优先级 | 文件与位置 | 问题和原因 | 推荐修改 / 本轮范围 | 风险与预计收益 | 状态 |
| --- | --- | --- | --- | --- | --- |
| UX-01 / P1 | `lib/editor/controller.dart:setSelection`；`editor.dart:_onControllerChanged`；`render.dart:RenderDocument.nodes/DocumentSurface` | 内容与选区共用通知，光标移动也重排和全篇扫描 | controller暴露内容修订号，选区通知不增加它；真正的内容通知增加修订号；surface使用可选修订号，普通选区只重绘。重复选区不通知。保留未提供修订号的调用者原行为；代码水平定位和details折叠仍按需要重排 | 中：漏失效会显示旧文本。用真实编辑器、代码块、折叠、修改/撤销/远端同步回归。收益是减少长文选区移动时的多余工作，不预先承诺帧率提升百分比 | DONE |
| UX-02 / P1 | `lib/editor/editor.dart:_refreshMarkBar/_onPan*/_autoScroll*` | 拖选中浮层抢占空间，取消残留手势状态，定时器滚动节奏不稳定 | 拖选结束才显示格式栏；增加取消清理；边缘滚动用调度帧和实际时间，速度随边缘距离渐进增长；失焦/销毁停止 | 中：拖选、表格范围、块移动共享路径。取消不得提交移动，键盘选区仍显示格式栏。收益是减少拖选干扰与滚动失控 | DONE |
| UX-03 / P1 | `lib/editor/editor.dart:_syncImeFromSelection/updateEditingValue/_ensureCaretVisible` | 候选窗口坐标可能在布局更新前读取，普通输入后缺少坐标刷新 | 帧后读取当前光标坐标并更新IME位置，与已有光标可见性调度协调；保持composing文本与提交顺序 | 中：输入是高风险路径。中文预编辑、折行、换行、提交/撤销测试；真实浏览器输入冒烟。真实Microsoft Pinyin若自动化不可用，明确记录限制 | DONE |
| UX-04 / P1 | `lib/editor/render.dart:EditorAppearance/EditorTheme/inline atom measure` | 标题压迫、间距缺节奏；主题漏失效；公式重复字号缩放 | 标题600字重并去负字距；段间12、列表项4、标题上28/连续16/下10；调色板参与相等与缓存判定；公式只缩放一次，fallback颜色走tokens | 中：尺寸改变影响命中/公式/折叠。像素与命中回归、字号/宽度切换、缓存测试。收益是排版层次与主题/公式一致性 | DONE |
| UX-05 / P1 | `lib/main.dart:_editorScroll/_formatBar`；新标题组件 | 页面标题弱于正文H1、长标题不换行、手机留白/工具栏空间紧张 | 标题独立可测试组件，字体与偏好协调；窄屏缩小外边距，格式工具栏在窄屏独立可滚动一行；桌面按钮分组与轻量悬停反馈 | 中：Enter进入正文和标题保存不能回归。标题长文/IME/Enter测试、390px真实界面。收益是主次明确、窄屏可编辑 | DONE |
| UX-06 / P1 | `lib/ui/dialogs.dart:_SettingsDialog.build/_appearanceSection/_aiSection`；新settings组件 | 固定容器溢出、混合设置类别、外观缺预览、AI请求阻塞全页 | 响应式设置壳、分类标题与说明；外观分组并实时预览；AI局部加载、失败重试；成功/失败反馈区分；保留local/cloud和权限判断 | 中：重排可能丢回调、改变设置范围。320/390/768/1440px、短视口、放大文字、挂起请求回归。收益是设置可发现、手机可用、偏好易判断 | DONE |
| UX-07 / P2 | `lib/ui/theme_tokens.dart:toMaterialTheme`；`main.dart:theme/darkTheme` | Material默认控件与自绘界面形状、颜色、字级不一致 | 明确表单、按钮、chip、switch、菜单/对话框主题，使用现有角色色与中文fallback；保留清晰焦点和合理点击面积；浅深色同时验收 | 中：全局主题影响其它对话框。对比度与组件状态测试、浅深色截图，保留警告/错误语义。收益是整体视觉统一 | DONE |
| UX-08 / P2 | `e2e/`；测试与本文验证记录 | 主观舒适感不能只由widget测试宣称改善 | 新本地UI fixture复用生产组件与真实编辑器；Playwright真实键鼠、长文输入/拖选/取消、设置导航、浅深色与窄屏截图；记录布局次数和浏览器帧表现；完整Flutter测试/分析/Windows构建 | 低：测试夹具可能和生产接线漂移，需直接复用生产组件并检查集成。收益是可复查与后续回归基线 | 自动化完成；Win32 门禁待 CI |

上述 `lib/` 相对 `clients/mica_flutter/`。

## 顺序与并行归属

1. 主agent落盘本方案，统一参数和接口；再启动开发。
2. 并行A：正文排版agent独占 `render.dart`（含DocumentSurface）及新增排版/渲染快路径测试。
3. 并行B：编辑交互agent独占 `controller.dart`、`editor.dart` 及新增交互/IME测试。A/B约定接口为controller的 `contentRevision` 和surface可选 `contentRevision`；代码/折叠依赖选区的布局失效由A处理。两者不得编辑对方文件。
4. 并行C：设置agent独占 `dialogs.dart`、新设置壳/预览组件、ARB与生成本地化、新设置测试。主agent负责在 `main.dart` 添加import。
5. 主agent独占 `theme_tokens.dart`、`main.dart`、新页面标题组件、浏览器fixture/脚本和本文，做集成、验证、复审、提交推送main。
6. 本轮不发版、不部署；用户再次明确发版时走完整发版流程。

## 验收

- 内容不变时，普通段落的光标/选区移动不增加排版次数；内容修改、撤销/重做、reconcile、字号/宽度/调色板变化仍正确更新。
- 代码光标水平可见性、details展开、表格、图片、数学、Markdown复制与粘贴继续通过已有测试。
- 拖选过程中不出现格式栏，松开后出现；取消后不继续滚动，不提交块移动；键盘范围选区不受影响。
- 中文预编辑不重复、不丢字，候选定位读取新布局；字号增大时公式只按同一比例增大。
- 长页面标题换行；Enter与候选确认正确；手机正文和设置内容不横向溢出（代码/表格自身水平滚动允许）。
- 设置分类、返回/关闭在320px、390px、短视口与文字缩放下可见；偏好立即可操作，不等待AI请求。
- 预览使用与真实编辑器一致的排版参数；浅/深色下正文、表单、选中态、错误反馈可读。
- `flutter test`完整通过，`dart analyze`无新增错误或警告，Windows构建完成；网页截图与实测记录保存于本目录下的验证说明/构建产物，避免把私有生产文档写入测试fixture。

## 参照与范围边界

- [Notion 页面样式](https://www.notion.com/help/customize-and-style-your-content)：少量字体/字号/页宽选项和上下文格式工具。
- [AppFlowy editor style](https://github.com/AppFlowy-IO/AppFlowy/blob/5cf3a365dec0d59f64bad1ee4bb1050471a39b93/frontend/appflowy_flutter/lib/plugins/document/presentation/editor_style.dart)：同为Flutter，正文与标题层级独立设计。
- [AFFiNE 设置导航](https://github.com/toeverything/AFFiNE/blob/6bb64575b1013d64df0382375541901244e8052d/packages/frontend/core/src/desktop/dialogs/setting/setting-sidebar/index.tsx)：设置作用域分组。

本轮不增加字体下载器、主题编辑器、封面图库或光标位移动画。现有中文字体只有Regular，调整字号/字重不等于已经获得真实多字重字体；新增字体资产需要另外评估授权、体积和实际收益，当前保留已许可字体与系统fallback。手机触摸选择/滚动若实测发现冲突，应先复现并在当前手势架构中修正，不凭代码推断删除功能。

## 实施与验证记录

UX-01～UX-07 已实现并通过针对性回归与浏览器实测；UX-08 的自动化脚本与 CI 门禁已实现，Windows 原生剪贴板复测受当前桌面环境限制，等待 CI Windows 门禁。微软拼音实际候选窗口位置仍需人工验收，不能由 CDP 预编辑测试代替。

结果与截图见 [UI_EDITING_EXPERIENCE_VALIDATION.md](UI_EDITING_EXPERIENCE_VALIDATION.md)。本轮保留原有字号/页宽偏好，未新增依赖、未改数据模型、未发版。
