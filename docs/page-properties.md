# 页面属性 / 标签(page properties / tags)

> 状态:M1 进行中(2026-07-22)。这份文档记**决策与边界**,尤其一个动了红线的取舍;实现细节看代码(`crates/markdown/src/properties.rs` 权威 + Dart 镜像)。

## 一句话

页面属性 = 把文档 **front matter** 从「不透明字符串」升级成「可编辑的结构化键值」。front matter **仍是唯一权威**(不加表、不加 object_type、不加第二份要同步的表示);属性是它的**懒解析视图 + 外科式写回**。

## 为什么这么做(调研依据)

拍板前扒了两组同类真实源码(CLAUDE.md #6):

- **md 权威系(Obsidian / Logseq / Outline)**:属性就是 front matter 解析成结构化。一致证据——**结构化编辑下 YAML 不可能字节保真**:Obsidian 原生 `processFrontMatter` 重排键序、删注释、改引号;Logseq 文件模式输出也是规范化子集。它们都**选择规范化输出**而非保真。tags 都只是「一个特殊属性」(值是 list + 进链接索引),不是独立维度。
- **关系库系(AppFlowy / AFFiNE / siyuan)**:给出**边界铁证**——「带类型、可筛选排序的**数据库视图**」与「markdown 权威 + round-trip」**根本不相容**。连和我们一样把一切存进 Yjs 文档的 AFFiNE,都拒绝让数据库 round-trip markdown(markdown 适配器 `toBlockSnapshot: {}` 空实现,单向有损导出)。三家全靠**把数据库节点从 markdown 权威里豁免**化解。

结论:页面属性走 md 权威系(契合我们约束);**数据库视图是另一件事**,见下「不做什么」。

## 红线动作:round-trip 不变量降级(已获用户批准 2026-07-22)

lessons.md 的 round-trip 是红线。front matter 存为不透明字符串时是**字节级保真**。一旦提供结构化编辑,被编辑的那个键**必须**重新序列化 YAML,字节保真守不住。

**降级**:front matter 的 round-trip 不变量从**「任意 YAML 字节稳定」** → **「规范化 YAML 子集稳定」**。这跟 mica 正文 markdown 早就采用的「输出规范化子集、round-trip 是子集不变量」**同构**,不是新妥协,是同一条已有原则延伸到 front matter。

**用外科式写回把损耗关进最小笼子**(比 Obsidian 更诚实一档):
- 编辑一个键 = **定点替换该键的行**,其余键的注释/键序/引号/空白**逐字不动**。
- 只有真正改动的键承受规范化;未触键零损耗。
- Obsidian 是整段重序列化(全丢注释/键序),我们不。

**我们守住并测试的不变量**(`properties.rs` 单测):
1. 编辑 key A **绝不**改动 key B 的原始字节;
2. `parse ∘ render` 稳定:重新渲染一个解析出的值再解析回来,得到同一个 `PropertyValue`。

## 范围(M1)与边界

**M1**:
- Rust 权威 `crates/markdown/src/properties.rs`:解析扁平子集 + 类型推断 + 外科 upsert/remove。
- 类型小封闭集(抄 Obsidian):`Text / Number / Checkbox / Date(YYYY-MM-DD) / List`,从 YAML 标量**推断**,无 per-key schema。
- `tags` = 一个 `List` 值的属性,不是独立类型;其列表项(M2)喂**现有** ref/backlink 索引。
- Dart 镜像 + 页头属性面板 UI(看/改/增/删),tags 渲染成 chips。

**支持的扁平子集**(即 Obsidian Properties 那档):顶层 `key: 标量`、flow list `key: [a, b]`、block list(`key:` 后跟缩进 `- item`)。**更复杂的**(嵌套 map、多行 block scalar、锚点)**原样保留、不作为可编辑属性露出**——它还是它原来的不透明字节。

**M2**(未做):tag 可点可搜(派生索引复用 ref 基建)、属性筛选、按属性排序。派生索引是**可丢弃缓存**(front matter 是唯一权威,索引脏了重建)——不破双表示红线。

**注定不做 / 另立项**:
- **数据库视图(Notion database:带类型列、筛选、排序、看板/画廊、relation、rollup)**。这要么把类型编码进 markdown(=第二份要同步的表示,破红线),要么像 AFFiNE/siyuan/AppFlowy 那样**把数据库节点从 markdown 权威里豁免出去**(散文仍 md 权威,数据库是 yrs 里一座带 id 的类型化孤岛、markdown 只单向导出)。后者是可行的**独立大决策**,不属于「页面属性」,单独立项再拍。
- **天花板诚实(Logseq 教训)**:文本权威 + 结构化查询有上限;不为假想的重度关系查询预抽象。

## 相关红线/原则

- CLAUDE.md #1 双表示红线:属性是 front matter 的派生视图,不是第二份权威;查询索引是可丢弃投影。
- CLAUDE.md #2 Rust-first:`properties.rs` 是权威,Dart `properties.dart` 镜像(同 marks.dart 纪律,两端必须同步)。
- CLAUDE.md #4 方言/round-trip:本次把 round-trip 不变量在 front matter 上从字节保真降为规范化子集稳定(见上)。
