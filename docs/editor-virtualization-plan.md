# 长文档虚拟化设计（render.dart layout 虚拟化）

> 2026-07-23。调研 CodeMirror 6 / AppFlowy / ProseMirror / Lexical / AFFiNE 后定稿。
> 目标:大文档每次击键只重排"改动块 + 视口带 + caret/IME 块",其余复用缓存/估算,
> 不破坏 caret / 选区 / IME / scroll-to-block / find。**这是设计与计划,落地留新会话。**

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

## 根因(就一个 bug)
`render.dart` 的 `performLayout`(约 919–925)**每次都把所有块的 `TextPainter` dispose 重建**:
```
for (final l in _layouts) l.painter.dispose(); _layouts.clear();
```
而 **paint 侧其实早有视口裁剪**(`_nodeVisible` / `_cullSlack=600`,约 1405/2189)。所以要做的
只是"让 layout 侧也虚拟化",跟上 paint。

## 数据结构(CodeMirror `HeightOracle` 的最小对应)
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
- **Phase 2(真虚拟化)**:估算高模型 + 屏外块**跳过** `TextPainter`(只用估算),`totalHeight`
  估算→测量自愈。
- **Phase 3**:offset→Y 前缀和 + caret 按需强排 + scroll/find 估算跳+沉降。
- **验证**:大档基准(万级块/巨块击键延迟)、caret/选区/scroll-to/find/IME 回归(桌面 integration_test
  + web playwright 截图)。红线:round-trip 不受影响(纯渲染层改动,不碰文档模型)。

## 参考
- CodeMirror guide(viewport + height 估算 + 视口外无 coords):https://codemirror.net/docs/guide/
- CodeMirror height map 源码:https://github.com/codemirror/view/blob/main/src/heightmap.ts
- AppFlowy 列表渲染(反例,widget-per-block 的代价):`appflowy-editor` `page_block_component.dart` / `editor_scroll_controller.dart`
- ProseMirror 作者"不做 viewporting":https://discuss.prosemirror.net/t/improving-performance-loading-on-scroll/4972
- 现状代码:`clients/mica_flutter/lib/editor/render.dart`(dispose-all @~919–925;paint 裁剪 `_cullSlack=600`)
