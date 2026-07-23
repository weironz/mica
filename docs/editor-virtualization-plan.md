# 长文档虚拟化设计（render.dart layout 虚拟化）

> 2026-07-23。调研 CodeMirror 6 / AppFlowy / ProseMirror / Lexical / AFFiNE 后定稿。
> 目标:大文档每次击键只重排"改动块 + 视口带 + caret/IME 块",其余复用缓存/估算,
> 不破坏 caret / 选区 / IME / scroll-to-block / find。

## 状态一览(2026-07-23,先看这里)——性能目标已闭环 ✅

**目标(大档每击键只做改动块的活)已达成**,走的是**架构干净支持、不变量不破、可离线验证**的
等效路线,而非原设计的"真·视口虚拟化"(后者被两条架构约束挡住,见下)。三刀叠加后,每击键
= **O(改动块)真推导 + O(N) 平凡重定位**。

| 项 | 状态 | 出处 |
|---|---|---|
| Phase 1:每块 painter 缓存(干掉 dispose-all/rebuild-all,未变块跳过 `layout()`) | ✅ 已落地 | da25075,`test/painter_cache_test.dart` |
| 代码高亮记忆化(未变代码块不再每击键重新分词) | ✅ 已落地 | 7fe1997,`test/code_span_memo_test.dart` |
| **Phase 2:整块 layout 复用 + 重定位(未变块跳过 marks/span/高亮/rect 全部推导,只 shiftBy)** | ✅ **已落地** | **b750d88,`test/layout_reuse_test.dart`** |
| 真·视口虚拟化(屏外**跳过**排版 + 估算高 + 前缀和 + caret 按需强排 + scroll/find 沉降) | ⏸️ **有意不做**(仅万级块/超长档才需要,且要架构级改动——见「两条架构约束」);剧本留档待需 | 下方 S0–S3(参考) |

> **闭环说明**:原设计想靠"屏外跳过排版"省下 O(N) 那部分,但两条架构约束(performLayout 拿不到
> 滚动偏移 + 编辑器不自管视口)让它做不到,且失败模式是离线测不到的静默几何损坏。Phase 2 改成
> **整块 layout 复用**:**保住"每个 `_layouts[i]` 都有真 painter+真几何"不变量**,只是"复用而非跳过",
> 于是既拿到"未变块零推导"的主要收益,又**可被离线 widget 单测(跑真实 layout/paint/hit-test)完整
> 把关**——不需要那套交互验证闸门(那是给"屏外无 coords"的真虚拟化用的)。剩下 O(N) 的重定位只在
> **万级块**才可能成为瓶颈;真到那天,再按下方 S0–S3 上"自管视口/paint 期惰性成形"。
>
> **本文档下半部(根因/数据结构/新 layout pass/分期剧本)是原始设计与"真虚拟化"路线的参考**,
> 其中部分假设已被「实测:两条架构约束」修正——凡冲突处以那节为准(逐处已就地标注 ⚠️)。

## 决策:CodeMirror 式单画布虚拟化,不改 widget-per-block

两大流派,分水岭是**谁拥有 caret/选区**:
- **contenteditable 系**(ProseMirror/Lexical/Slate/AFFiNE 文档模式):选区交给浏览器 → 全文必须在 DOM → **不虚拟化**(PM 作者 Marijn 明确:"ProseMirror doesn't do viewporting … puts the entire document in the DOM")。
- **自管渲染+选区系**(CodeMirror 6):**能**虚拟化——viewport-only 布局 + 全文 height map(先估算、绘制时测量)。

**mica 在 CodeMirror 阵营**:已经自绘、自画 caret、自走 `TextInputClient` IME。所以单
RenderBox **不是要重写的包袱,恰是能虚拟化的前提**。CodeMirror 6 就是"单自管表面 + 每行
height map"可行的存在证明。

**为什么不学 AppFlowy(widget-per-block + `ScrollablePositionedList` 窗口化)**:它白拿了
虚拟化,但代价明确且已在其源码坐实——**没有绝对滚动偏移、没有精确总高、只能按 index 跳、
巨块内部不虚拟化**(`jumpTo` 走 `ItemScrollController` 的 index,`offsetNotifier` 只是累积
delta)。为它抛弃 mica 的自绘核心 + marks-over-plaintext + 自画 caret,得到一个几何保真度
更差的版本——不划算。

## 根因(就一个 bug)—— ✅ 此根因 Phase 1 已修(da25075)
> 下面描述的 dispose-all 已删除;`_painterCache` 复用替代。保留作背景。行号是 Phase 1 前的旧值。

`render.dart` 的 `performLayout`(旧约 919–925)**每次都把所有块的 `TextPainter` dispose 重建**:
```
for (final l in _layouts) l.painter.dispose(); _layouts.clear();
```
而 **paint 侧其实早有视口裁剪**(`_nodeVisible` / `_cullSlack=600`,现 @1506/1508)。所以要做的
只是"让 layout 侧也虚拟化",跟上 paint。**⚠️ 但"让 layout 侧虚拟化"这句本身有前提问题——见
「两条架构约束」#1:`performLayout` 拿不到视口,layout 侧无法像 paint 那样按可视带跳过。**

## 数据结构(CodeMirror `HeightOracle` 的最小对应)
> **状态**:#1(每块缓存)✅ Phase 1(指纹用实际 TextSpan,比"marks hash"更稳)+ Phase 2(整块
> layout 缓存 `_layoutCache`)。#2(dirty 集)✅ **由 identity 实现**——controller 只重赋值不原地改
> text/data,故 `identical` 即"未变"(比"标脏"更省:无需布线,零漏标)。#3–#4(估算高 / 前缀和)
> **仅"真·视口虚拟化"才需要**,现方案不跳过排版故不需要;真要走那条再实现(见顶部闭环说明)。

1. **每块 layout 缓存**,按 block id 键:`TextPainter`(或 atomic 布局)、`boxHeight`、
   `contentLeft`/`textWidth`,以及**算它时的输入指纹**(text、marks hash、maxWidth、外观 rev、
   代码语言、quote/li 缩进)。把"dispose 全部 + 重建全部"换成"**只在输入变了才逐出+重算**"。
2. **dirty 集**(block id):编辑应用把被改块标脏;**宽度变**把全部标脏(宽度喂每块换行);
   外观/主题变全脏;IME 合成块每帧标脏。
3. **每块估算高**(未排版块):`estHeight = topGap + ceil(charCount / estCharsPerLine) * lineHeight`,
   `estCharsPerLine = floor(textWidth / avgCharWidth)`。atomic 块(图/分隔/公式/表)用已缓存
   的 intrinsic 高或 per-kind 默认。
4. **累计 Y 前缀和** + **`totalHeight = Σ(有缓存用精确,否则用估算)`**:RenderBox 据此报稳定的
   `size.height`/滚动范围,不必排版全文。前缀和即 `offset→block` / `block→Y` 映射(块数极大时
   上 Fenwick,普通文档一个 `List<double>` dirty 时重算即可)。

## 新的 layout pass(替换现 `performLayout`)
> ⚠️ **此节的核心前提已被证伪,勿照抄**:第一条"像 paint 一样算可视带"在 `performLayout` 里**做不到**
> ——见「两条架构约束」#1(视口只在 paint 时知)。真要落地得改成"paint 期惰性成形"或"编辑器自管视口",
> 触发点不在 `performLayout`。下面几条(复用缓存 painter、估算→测量自愈)本身仍成立,只是**驱动位置变了**。

- 像 paint 一样算可视带(`clip` ± cull slack)。
- 走块累计 Y。每块:若**在带内 OR dirty OR caret/合成块** → 真跑 `TextPainter.layout`(输入没变
  就复用缓存 painter),把**精确高**写回缓存 + 前缀和;否则用**缓存或估算**高、**完全跳过**
  `TextPainter`。
- 估算被测量替换时,按 delta 调 `totalHeight`(CodeMirror 做法;超过小 epsilon 才回流滚动条,
  避免抖动;高度量化到 0.1px 抑噪)。

## 五个正确性雷区(每个对应一处同类实证)
1. **未排版块里的 caret/选区**:`coordsAtPos` 式查询必须**按需强排目标块**再返回几何——**绝不**
   从估算读几何(CodeMirror 铁律"视口外无 coords";这里翻成"先排它")。`caretRectFor(node,offset)`
   → 确保该块(及经前缀和拿到其 Y 的上下文)已排版,再测量。mica caret 本就走 painter,这是
   "按需测量"、非新机器。
2. **scroll-to-block / find**:用前缀和的**估算 Y** 跳过去,**到达时**带内排版自愈上下高度、滚动
   自校正(CodeMirror 估算→测量)。首次到远处允许一次小"沉降"——已知代价,且**严格优于**
   AppFlowy 的纯 index 跳(我们至少有估算)。
3. **跨屏选区**:选区**模型是 offset**(mica 本就这么存),只有**画**高亮要几何,而只画可视带 →
   跨屏选区模型正确、屏外不画即可。**别去算屏外选中块的 rect。**
4. **IME/合成**:合成块每帧必须有真 `TextPainter`(标脏 + 合成期"永远排版"),IME 候选窗需精确
   caret rect。这是唯一绝不能吃估算的块。
5. **单个巨块**:与 AppFlowy 不同,mica 自己排文本、**日后可**子虚拟化巨代码块;v1 先把
   聚焦/可视巨块整块精确排、其余估算,仅当单块成瓶颈再回来。

## 分期落地
- **Phase 1(最大收益、最低风险)✅ 已落地(da25075,2026-07-23)**:每块 layout 缓存,
  **干掉 dispose-all/rebuild-all**。实现比原设想更稳:复用"输入指纹"直接取**实际 TextSpan +
  布局宽度**(span 已含 text/marks/已解析样式,span 相等即输入相等,免手列字段——躲开 1 号
  雷区)。文本 painter 归 `_painterCache` 所有,只有原子渲染器 painter 逐帧重建;字体加载
  (`_fontsDirty`)一次性清缓存强制重排;含 fold 的块存而不复用(异步栅格竞态);pass 末按
  `seenTextIds` 逐出失效条目。回归见 `test/painter_cache_test.dart`。**注:本期只做了缓存复用
  (跳过未变块的 layout()),尚未做"屏外块跳过排版/估算高"——那是 Phase 2。**
- **Phase 2(真虚拟化)❌ 未做**:估算高模型 + 屏外块**跳过** `TextPainter`(只用估算),`totalHeight`
  估算→测量自愈。**⚠️ 落地前提见「两条架构约束」——不能在 `performLayout` 里做。**
- **Phase 3 ❌ 未做**:offset→Y 前缀和 + caret 按需强排 + scroll/find 估算跳+沉降。
- **验证 ❌ 未做**:大档基准(万级块/巨块击键延迟)、caret/选区/scroll-to/find/IME 回归(桌面 integration_test
  + web playwright 截图)。红线:round-trip 不受影响(纯渲染层改动,不碰文档模型)。

## 实测:两条架构约束(动手后确认,2026-07-23)——决定"真虚拟化"是独立架构项

动 Phase 2 前扒了实况,两条硬约束改写了原设想,**必须先读**:

1. **`performLayout` 拿不到滚动偏移。** RenderDocument 被布在**全高**、外层 ScrollView 在 **paint**
   时才靠 `offset.dy` + `canvas.getLocalClipBounds()` 给出可视带(`render.dart:1512-1516`,
   `_visTop/_visBottom`)。所以"layout 阶段跳过屏外块"在本架构**做不到**——要么让编辑器**自管视口**
   (RenderAbstractViewport,大改),要么把成形挪到 **paint 期惰性做** + 估算→测量自愈(有帧内抖动、
   易进无限 relayout,需极小心)。这不是"一次性安全改动"。
2. **`EditorNode.text/data` 原地可变**(`model.dart:34-35`;controller 单列表原地改,`controller.dart:46`)。
   于是**没有廉价且正确的"块变没变"信号**:shadow 比对要逐块深拷贝(贵),而 Phase 1 正是靠**每次重建
   span**绕开这点。要跳过 span 构建就得给每个 controller 变更点加 **rev 计数**(landmine,漏一个=显示旧内容)。

**结论**:Phase 1 已抓住主导成本(shaping);架构**干净支持**的最高性价比补充是**记忆化代码高亮**
(未变代码块不再每击键重新分词)——**✅ 已落地 7fe1997**,key=(code,language,base) 完整永不 stale,
回归 `test/code_span_memo_test.dart`。**"真·视口虚拟化"(自管视口 or paint 期惰性成形)是独立架构项,
ROI 只在万级块/超长档才显著**,按下方剧本 + 交互验证在专门会话做;非大档场景可先不做。

## Phase 2/3 实施剧本(新会话直接照做,2026-07-23 定)

> 决策:Phase 2/3 **专开一个会话做**,且必须在**能跑起来的应用上做真实交互验证**才算完。
> ⚠️ 先读上一节两条架构约束:S0/S1 的"屏外跳过排版"**不能在 `performLayout` 里做**——要落地得先选
> "自管视口"或"paint 期惰性成形"其一,剧本的 `_ensureLaidOut`/前缀和/估算仍适用,但触发点在 paint/查询期。
> 理由(实测判断,非泛化谨慎):Phase 1 保住了"每块都排、每个 `_layouts[i]` 都有真 painter"
> 这条不变量,所以约 80 处 `_layouts[i]` 消费点原样能用;**Phase 2 故意打破它**(屏外块只有
> 估算高、无成形 painter),于是**任何几何查询都可能拿到没排过的块**——`caretRectFor`(现
> `render.dart:2812`)、`positionAt` 命中测试(`2837`)、`_paintSelection`(`2553`)、scroll-to、
> find、表格/图片热区。**所以 Phase 2 离不开 Phase 3 的"按需强排目标块",两者是一次耦合改动,
> 落在那 ~80 个点上,不是一处。** 五个正确性雷区全在这一刀里。失败模式是**静默** caret/IME/
> 滚动几何损坏,离线 widget 单测查得到 `caretRectFor` 的 rect,却**查不到真实 IME 合成 / 滚到
> 远块的沉降 / 跨屏拖选**——正是雷区所在。离线全绿≠对,那是 `lessons.md` 的"测试真空通过"。

### 分阶段顺序(每步可编译、可验证,再进下一步)

- **S0 基础设施(不翻开关,零几何风险)**:加**估算高模型** + **offset↔Y 前缀和** + **按需
  强排** `_ensureLaidOut(nodeIndex)`,但 `performLayout` **仍照排全部块**。此时前缀和/估算是
  **并行计算**,和真实 layout 并存做一致性断言(每块 `估算高` vs `实排高` 记录,debug 下断言
  偏差在阈内)。离线可测:前缀和 `blockAtY`/`yOfBlock` 往返、`_ensureLaidOut` 幂等。
  - `estHeight = topGap + ceil(charCount / estCharsPerLine) * lineHeight`,`estCharsPerLine =
    floor(textWidth / avgCharWidth)`;atomic 用已缓存 intrinsic 高或 per-kind 默认。
  - 前缀和:普通文档一个 `List<double>`(dirty 时重算);块数极大再上 Fenwick。
  - `_ensureLaidOut(i)`:若第 i 块本 pass 未成形,就地按文本管线排它(复用 `_painterCache`),
    写回精确高 + 修正前缀和。**几何查询的唯一合法入口**——"视口外无 coords"翻成"先排它"。

  **S0 第一刀 = 抽 `_shapeTextBlock`(纯重构,行为零变,715 测试当闸)——已勘好边界,照抄即可:**
  - **抽出体**:`render.dart` 现 **1041(`final style = _appearance.applyTo(`)→ 1423(`if (isCode)` 的收尾 `}`)**,
    整段文本管线成形逻辑。**1425–1427 的 `_layouts.add / y += / prevKind` 留在循环**,方法只 `return layout;`。
  - **方法签名**(放 `performLayout` 之后):
    ```dart
    _NodeLayout _shapeTextBlock(EditorNode node, double y, double maxWidth, {
      required String? nodeAlert, required bool isAlertHead,
      required List<int> numberedCounters,   // 按引用改(ordinal 计数)
      required bool caretMoved, required DocSelection? sel,
      required Set<String> seenTextIds,      // 按引用改 + 写 _painterCache
    }) { ...抽出体... return layout; }
    ```
    循环里替换为 `final layout = _shapeTextBlock(node, y, maxWidth, nodeAlert: nodeAlert, isAlertHead: isAlertHead, numberedCounters: numberedCounters, caretMoved: caretMoved, sel: sel, seenTextIds: seenTextIds);`
  - **两个必须守住的隐形不变量(错一个就静默损坏)**:
    1. **`quoteAlert` 归属计算(现 1012–1024)必须留在循环、在原子派发之前**——它对**每个**块(含 atomic)更新
       跨块 callout 分组状态;只把结果 `nodeAlert`/`isAlertHead` 传进方法。
    2. 方法内 code 水平自动滚动那段用 **`sel.focus.node == _layouts.length`** 判"当前块"(尚未 append,
       故当前 index == `_layouts.length`)。方法**必须在 `_layouts.add` 之前调用**,该等式才成立——保持现有调用顺序即可。
  - 方法用到的都是字段(`_appearance`/`_painterCache`/`_codeScroll`/`_inlineAtomRenderers`),无需再传。
  - 抽完:`flutter test` 全绿(行为不变)即算 S0 第一刀成。之后 S1 只是"给 `_shapeTextBlock` 的调用加跳过条件 +
    让 `_ensureLaidOut` 调它",是一个小 diff。
- **S1 翻开关(风险所在,单独一步单独一测轮)**:`performLayout` 只对**可视带(clip ± cull
  slack)∪ dirty ∪ caret/IME 合成块**跑 `TextPainter.layout`,屏外块**只填估算高 + 前缀和,
  完全跳过 painter**。`totalHeight = Σ(有缓存用精确,否则估算)`,`size.height` 据此稳定。
  估算被测量替换时按 delta 调 `totalHeight`(超小 epsilon 才回流滚动条防抖;高度量化 0.1px)。
- **S2 消费点审计(和 S1 同一 PR,缺一即静默损坏)**:每个读 `_layouts[i].painter`/几何的入口,
  前面插 `_ensureLaidOut(i)`。至少覆盖:`caretRectFor`(2812)、`positionAt`(2837)、
  `_paintSelection`(2553,跨屏选区**只画可视带**,屏外选中块**不算 rect**——选区模型是 offset,
  见雷区 3)、code 水平自动滚动取 caret x、find 命中跳转、表格/图片/代码工具条热区(`*At(local)`
  系列)。**paint 侧**已按 `_nodeVisible`(1508)裁剪,天然只碰可视块,基本安全,但确认它读
  的 boxTop 来自前缀和而非旧全排。
- **S3 scroll-to / find 沉降**:用前缀和的**估算 Y** 跳过去,到达时可视带排版自愈上下高、滚动
  自校正;首次跳远允许一次小沉降(已知代价,仍**严格优于** AppFlowy 纯 index 跳)。

### 五雷区 → 落到具体代码(逐条盯)

1. **未排块里的 caret/选区**:`caretRectFor`/`positionAt` 进来先 `_ensureLaidOut(pos.node)`,
   **绝不**从估算读几何。
2. **scroll-to/find**:估算 Y 跳 + 到达沉降(S3)。
3. **跨屏选区**:选区模型是 offset(本就如此),只有**画**要几何且只画可视带 → 屏外选中块
   **别算 rect**;`_paintSelection` 里屏外块 skip。
4. **IME/合成**:合成块每帧必须真 painter(标脏 + 合成期"永远排版"),候选窗要精确 caret rect。
   **唯一绝不能吃估算的块。** 对应 `TextInputClient` 合成路径 + `caretRectFor`。
5. **单个巨块**:v1 先把聚焦/可视巨块整块精确排、其余估算;单块成瓶颈再子虚拟化。

### 交互验证闸门(这些没过 = 没完,不许发版)

离线 widget 单测(扩 `test/painter_cache_test.dart`:前缀和往返、`_ensureLaidOut` 幂等、屏外块
删增不崩)是**必要不充分**。**签收必须在跑起来的应用上过下列真实交互**(desktop
`integration_test/` + web playwright-cli 截图,`justfile` 有 recipe):

- [ ] **万级块大档**:PageDown/滚动到中段/末段,caret 点击落点准、无错位、无空白带。
- [ ] **滚到远块**:`scrollToBlock`/find 跳到文档 90% 处,沉降后 caret 与目标块对齐(截图比对)。
- [ ] **跨屏拖选**:从可视首块拖到需滚动才到的块,选区 offset 正确、可视部分高亮对、导出文本对。
- [ ] **真实 IME 合成**:中文输入法在长档中段边打边选候选,候选窗跟随 caret、合成串不错位
      (web playwright 模拟 composition 事件 + 桌面手测截图)。
- [ ] **巨代码块**:单块几千行,块内 caret/滚动/行号 gutter 对齐。
- [ ] **基准**:万级块档单次击键 layout 耗时(S1 前后对比,记进本文档)。

### 红线
纯渲染层,**不碰文档模型/CRDT/序列化**,round-trip 不变量零改动。`_painterCache`(Phase 1)
的所有权/字体失效/fold 不复用/prune 规则**继续适用**,`_ensureLaidOut` 复用它,别另起炉灶。

## 参考
- CodeMirror guide(viewport + height 估算 + 视口外无 coords):https://codemirror.net/docs/guide/
- CodeMirror height map 源码:https://github.com/codemirror/view/blob/main/src/heightmap.ts
- AppFlowy 列表渲染(反例,widget-per-block 的代价):`appflowy-editor` `page_block_component.dart` / `editor_scroll_controller.dart`
- ProseMirror 作者"不做 viewporting":https://discuss.prosemirror.net/t/improving-performance-loading-on-scroll/4972
- 现状代码:`clients/mica_flutter/lib/editor/render.dart`(Phase 1 后:`_painterCache` 缓存复用,
  dispose-all 已去除;paint 裁剪 `_cullSlack=600` @1506、`_nodeVisible` @1508;几何入口
  `caretRectFor` @2812 / `positionAt` @2837 / `_paintSelection` @2553)
