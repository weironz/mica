# 键盘快捷键(Keyboard shortcuts)

Mica 全部键盘快捷键的**权威清单**。加/改快捷键时三处必须同步(否则会漂):

1. 实现:`clients/mica_flutter/lib/editor/editor.dart` 的 key handler(编辑器内)
   + `clients/mica_flutter/lib/main.dart` `_appShortcuts()`(应用级)。
2. 设置面板:`clients/mica_flutter/lib/ui/dialogs.dart` 的 `_shortcutsSection`。
3. 本文档。

> macOS 上一律用 **⌘** 代替 **Ctrl**(代码里 Control/Meta 两个变体都绑了)。

## 应用级(`main.dart` `_appShortcuts`)

| 快捷键 | 功能 |
|---|---|
| `Ctrl + N` | 新建页面 |
| `Ctrl + F` | 页内查找(当前文档内) |
| `Enter` / `F3` | 查找栏内:跳到下一个匹配(环绕) |
| `Shift + Enter` / `Shift + F3` | 查找栏内:跳到上一个匹配(环绕) |
| `Ctrl + Shift + F` | 全工作区搜索 |
| `Ctrl + ,` | 打开设置 |
| `F2` | 重命名侧栏高亮的那一行(页面/文件夹),原地改名 |

> `Ctrl + ,` 会被中文输入法在 OS 层吞掉(标点切换),此时切英文输入法,或从菜单进设置。

> **页内查找/替换**:`Ctrl + F` 打开右上角查找栏(有选区则预填、大小写不敏感、实时显示
> `当前/总数`),`Esc` 关闭并把焦点还给编辑器。栏左侧的展开箭头(`▸`)打开替换行——
> 没有单独的 `Ctrl + H`(web 上 `Ctrl + H` 是浏览器历史,会被劫持)。「替换当前」替换后
> 前进到下一个,「全部替换」一步替换全文(单次撤销)。替换全部走控制器的
> `replaceRange`/`replaceAll`(和其它编辑一样的 `update_block` 路径,不新建第二份编辑表示)。

> **为什么改名是 F2 而不是双击**:侧栏树行注册 `onDoubleTap` 会让
> DoubleTapGestureRecognizer 占住手势竞技场(`hold`),于是**每一次单击**都要等
> `kDoubleTapTimeout`(300ms)才生效——为一个低频的改名,去税掉"单击打开页面/展开文件夹"
> 这条热路径。同类产品调研结论一致:AppFlowy(Flutter,同款约束)侧栏树**一个
> `onDoubleTap` 都没有**,还专门写了 200ms 节流把第二次点击**丢弃**,改名走 ⋯ 菜单 + F2;
> AFFiNE(React,`onDoubleClick` 零成本)**照样没用**双击,只给右键菜单 + 新建自动改名——
> 这条排除了"只是因为 Flutter 贵",说明是独立的 UX 判断。而且 Explorer/Finder/VS Code 里
> 树行双击的肌肉记忆是**打开**,不是改名。
>
> Mica 的改名入口因此是三个:**F2**、行的右键/⋯ 菜单 →「重命名」、以及新建页面/文件夹后
> 自动进入改名。三者都落到同一个**行内 TextField**(比 AppFlowy 的弹框/AFFiNE 的浮层更接近
> 原地编辑)。

## 编辑器 —— 格式

| 快捷键 | 功能 |
|---|---|
| `Ctrl + B` | 粗体 |
| `Ctrl + I` | 斜体 |
| `Ctrl + E` | 行内代码 |
| `Ctrl + K` | 链接 |
| `Ctrl + Alt + 1…6` | 标题 H1–H6 |
| `Ctrl + Alt + 0` | 转为正文(段落) |
| `Tab` / `Shift + Tab` | 列表项缩进 / 反缩进 |

> 标题走 Notion/Word 约定的 `Ctrl+Alt+数字`,不是 Typora 的裸 `Ctrl+数字`——web 端裸
> `Ctrl+1…9` 被浏览器占用(切标签页),应用根本收不到。

## 编辑器 —— 编辑

| 快捷键 | 功能 |
|---|---|
| `Ctrl + Z` | 撤销 |
| `Ctrl + Shift + Z` / `Ctrl + Y` | 重做 |
| `Ctrl + A` | 全选 |
| `Ctrl + C` / `Ctrl + X` / `Ctrl + V` | 复制 / 剪切 / 粘贴 |
| `Ctrl + Shift + V` | 粘贴为纯文本(不解析 Markdown) |
| `Delete` / `Backspace` | 选中整块(分隔线 / 图片)时删除该块 —— 点一下分隔线或图片即选中 |
| `/` | 斜杠命令菜单 |

## 表格(选中行/列/区域时)

| 快捷键 | 功能 |
|---|---|
| `Esc` | 取消区域选择 |
| `Ctrl + C` / `Ctrl + X` | 复制 / 剪切单元格(TSV + HTML) |
| `Delete` / `Backspace` | 清空选中单元格 |

## 右键菜单(非键盘)

- **正文**:复制 / 剪切 / 粘贴 / 粘贴为纯文本。
- **图片**:图片相关操作(替换、复制、删除等)。

## 输入即转换(Markdown 自动格式化)

在行首键入以下标记会即时把当前块转成对应类型(镜像 CommonMark 写法):

| 键入 | 转成 |
|---|---|
| `# `…`###### ` | 标题 H1–H6 |
| `- ` / `* ` / `+ ` | 无序列表 |
| `1. ` | 有序列表 |
| `- [ ] ` / `- [x] ` | 待办 |
| `> ` | 引用 |
| ` ``` ` | 代码块 |
| `---` | 分割线 |

行内(键入闭合标记即转换):`**粗**`、`*斜*`、`` `码` ``、`~~删~~`、`[文字](url)`、`$公式$`。

> 中文强调对 CJK 友好:`**加粗。**后文` 这类紧贴全角标点的强调能正确成对
> (见 `docs/commonmark-scoreboard.md` 方言扩展)。
