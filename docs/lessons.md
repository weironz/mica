# 踩过的坑(会反复咬的那种)

这份文档记的不是"怎么做",是**"这么做过,错了,代价是什么"**。CLAUDE.md 写规则,
这里写规则的来历——因为规则本身很容易被当成"大概是这么个意思"而绕过去,
而下面每一条都是绕过去之后真的付出过代价的。

新会话/新环境接手时,这份和 CLAUDE.md 一起读。

---

## 1. 双表示:op-model 快照 vs yrs CRDT

云文档在服务端有**两套表示**,这是本项目最反复咬人的一处:

1. **op-model** `document_snapshots`(jsonb block 模型)
2. **yrs CRDT** `document_yrs_base`(每次 push_update 折叠出的当前权威态)
   + `workspace_updates`(增量流)

yrs base 首次由 op-model 快照 lazy 构建(`sync::ensure_base_tx`),之后**单向**:
snapshot → yrs base,**没有反向折叠**。云会话切成纯 yrs append-log 之后,
云端编辑只进 yrs,`document_snapshots` 就**永久冻结在建文档时的初始态**。

### 读侧(已修)

症状:云页面在编辑器里看得见内容(走 yrs 渲染),但**导出空白、双击重开空白**——
因为那些路径读的是死快照。生产实证过:一篇文档 snapshot 是 2 块 / seq=1,
而 yrs_updates 有 21 条,解码 yrs base 得到 16 块真内容。

修法:`store::current_payload(db, doc)` —— 有 `document_yrs_base` 就物化它,
否则回退 op-model 快照。

> **红线:任何服务端"读文档"的 handler 必须走 `current_payload`,
> 不准直接读 `store::latest_snapshot`。** 否则空白问题原地复现。

### 写侧(已修,但教训更贵)

这颗雷当年就预言过"MCP 写了用户看不见",**后来在生产真的发生了**:
用户 MCP append 三次全返回 ok、`current_seq` 1→2→3、快照涨到 124 块,
而任何读取仍然是 75 块。

根因:`apply_derived_operations` 拿**没人读的快照**当"当前内容"推导 append 位置,
写完也只落回快照。修法落在这一处(它覆盖 REST ops / MCP markdown / WS 旧 op 三个调用点):
基线改从 yrs base 取,结果用 `doc.set_blocks` 作为**前向操作**写回 yrs,
update 进 `workspace_updates` 流并刷新 base。

> **刻意不做逐 op 的 yrs 映射——同一语义两套实现正是这个 bug 的成因。**
> `set_blocks` 让 yrs 状态按构造等于目标 payload,代价是每次 REST/MCP 写产生
> 全量 update、并发块级后写胜。这个代价是知情接受的。

### 还没收敛的部分

**两套表示仍然并存。** 新增任何写路径务必经 `apply_derived_operations`。
另外:**恢复历史版本绝不能"把旧状态当 update 应用"**——CRDT 是并集,不回退。
必须用 `set_blocks` 在当前 doc 上重建目标内容。

---

## 2. 不变量只写在客户端 = 没写

"folder 是唯一容器,page 是叶子"这条规则,最初只做在 Flutter 客户端
(`models.dart: canNestUnder`):UI 上不给你这个操作,拖拽也拒绝。
服务端**一行校验都没有**。

结果:Notion 导入按自己的逻辑造树,生产上落了 **137 个「页面挂子页面」**。
只要绕过客户端 UI(导入、MCP、直接调 REST),想造什么树就造什么树。

修法是三层,缺一层都不够:

| 层 | 做什么 | 为什么不能省 |
| --- | --- | --- |
| 存量修复 | migration `0011` 的 DO 循环 | 已经在库里的坏数据不会自愈 |
| API 校验 | `ensure_parent_accepts_children` | 给调用方 400 + 可读原因,而不是 500 |
| DB 触发器 | `views_parent_must_be_folder` | 兜住"以后有人加了新路径又忘了走 API 校验" |

> **教训推广:任何"产品规则",如果只有 UI 挡着,就等于没有。**
> 判断标准很简单——绕过 UI 直接打 API,能不能破坏它?能,就是没做。

### 变种:判定放在看不见证据的层

同一类病的另一个形态——不是"少做了一层",而是**做在了信息不足的那层**。

Dart 块层用 `lines[i].endsWith('  ')` 预判硬换行,再把行尾两空格换成
`\`+换行。但 **code span 可以跨行**:

```
a `code␠␠
span` b
```

那两个空格是**代码内容**,不是硬换行。块层逐行工作,根本看不到这个行尾
是否正落在一个 code span 内部——它在信息不足的位置做了判定,于是往代码
内容里注入了一个字面 `\`。

关键是:**这个 bug 补不出来。** 块层再怎么加特判也拿不到"我在不在 span 里"
这个信息(span 的另一半在下一行,还没读到)。Rust 早就把判定整体下移到了
行内层(`parse_inline_memo` 前置段),块层只负责**保留行尾空格当证据**
(`body`);Dart 没跟上,就成了双表示漂移。修法是搬层,不是打补丁:块层
改成一律 `\n` 拼接 + 保留行尾空格,`endsHard` 连同 10 处引用删除。

> **判据:如果一个判定所需的信息在当前层根本看不到,任何特判都是猜。**
> 正确动作是把判定移到能看见证据的层,并让上层负最小责任——**别销毁证据**。

---

## 3. 测试可以"真空通过"

这一条排在这里是因为它**让我误以为验证过了**,比 bug 本身更危险。

- **`sync_pg.rs` 那套「没有 DATABASE_URL 就 return」的模式会让测试真空通过。**
  第一次写 yrs 写侧的红测试时,它"通过"了——因为连不上库,函数直接 return。
  只有塞了一行 println 探针才发现连接根本没建立。
  **别把这种绿当验证过。** 断言之前先确认前置条件真的成立。
- **`cargo check` 不编译测试目标。** 改了公共结构体,`check` 全绿,
  `cargo test` 才暴露测试里的构造点少字段。发版前跑 `--all-targets`。
- **CI 只跑纯 crate 的测试**(无数据库)。带 DB 的路径没有 CI 兜底,
  改动必须自己验。
- **对抗复审会判错。** 图片解码那处 dispose 时序,复审明确判定"符合文档、没问题",
  **判错了**,而我采信了没实证。
  **凡是能被一个测试证伪的结论,不要靠读代码定论。**

### 补 fixture 的收益在暴露面,不在钉住的那个 case

2026-07-21 补了三个 conformance fixture(`38-hardbreak-spans` /
`39-bracket-spans` / `40-code-span-strip`),目标是钉住三处已确认的
Rust↔Dart 漂移。三处本身都不大(一处注入多余 `\`,一处丢链接,一处
tab 没剥)。

真正的收获是:`39-bracket-spans` 刚落盘,**round-trip 断言就炸出一个
不在任何清单上的序列化器 bug**——

```
[`a`](/x)   →  导出成  `a`      // 链接整个丢掉
```

根因在 `render_span` 选 mark 的排序键 `(start, 最宽)`:起止**完全相同**
时只能看谁在数组里靠前,而 `code` 分支是**终止性**的(直接 `pos = e`,
从不渲染 inner),它一旦赢得平局,同范围的 link 就再也没机会渲染。
`math`/`html`/`footnote` 三个分支同病。修法是加第三排序键
`renders_terminal`,让终止性 mark 平局必输、渲染到最内层。

`[`useState`](https://…)` 这种形状在技术文档里遍地都是,**这个才是真会
丢用户数据的**,而它不在清单上、也没人想到要去查它。

> **判据:挑 fixture 按「输入形状的多样性」,不是按「已知 bug 的清单」。**
> 一个 fixture 进来会同时喂进两套断言(双引擎逐字节比对 + round-trip),
> 收益主要来自你**没想到**的那一侧。清单清空不等于问题清空。

### 迁移怎么验

迁移跑错 = api 起不来,所以不能只靠读。可复用手法(不碰任何生产密码):

```sh
# 在生产 postgres 上开一次性 scratch 库,走 ssh 管道喂 SQL
ssh <node> 'docker exec -i mica-postgres-1 psql -U mica -d postgres \
  -c "CREATE DATABASE mica_migtest"'
# 装到待验迁移的前一个 → 灌违规 fixture → 确认 RED → 装待验迁移 → 断言
ssh <node> 'docker exec -i mica-postgres-1 psql -q -U mica -d mica_migtest' < migrations/00XX.sql
# 跑完 DROP DATABASE
```

另外:**`sqlx::migrate!` 是编译期嵌入的,新增迁移文件不触发 `mica-infra` 重编**。
加了迁移要 `touch crates/infra/src/db.rs` 强制重编,否则 `run_migrations` 还带着旧集合
(踩过:表不存在)。

---

## 4. Flutter:web 通过 ≠ 桌面通过

**Flutter debug build 跑 `assert`,release build 全部跳过。`flutter build web` 出的是 release。**

所以 web 上"功能正常"不代表桌面正常。做行内数学公式时,web(playwright)充分实测
点击弹框 + 编辑写回全过,推断桌面同源;重建桌面 debug 实测才发现**编辑写回红屏崩溃**——
`showDialog` 刚 pop、route 还在 deactivating 时同步碰 IME,触发 debug-only 断言。
web release 把它吞了,差点漏到发版。

- 编辑器 / 对话框 / IME 相关改动,**两端都要实测**。
- **dialog pop 之后碰 IME 或 render,一律延到 `addPostFrameCallback`。**
- 桌面 debug 崩溃走 `OutputDebugString`,重定向 stdout/stderr 抓不到,
  只有红屏 ErrorWidget 显示 assertion message(没有完整 stack)。

### 条件导入的结构性盲区

`local_offline_io.dart` / `local_offline_web.dart` 这种条件导入,**只有 dart2js 会解析 web 变体**。
往 IO 变体加了新方法忘了同步 web 桩,`flutter analyze` 和桌面构建**都不报错**,
只有 `flutter build web` 才炸——已经连挂了 4 个 release 才被发现。

对策:抽 `LocalOfflineApi` 抽象接口,两个变体都 `implements` 它(少一个方法就编译不过);
CI 加 `flutter build web --release` 关卡。

---

## 5. 图片解码:dispose 时序是承重的

**`ImageDescriptor.dispose()` 会让 codec 还需要的编码数据失效。**
在 `getNextFrame()` 之前调用它 → 每次解码都失败,报
`Codec failed to produce an image` → **应用里每张图都变成灰色占位框**。

而图片 URL 单独打开完全正常,所以极易误判成网络或存储问题。

正确写法(和 Flutter 自己的 `instantiateImageCodecWithSize` 一致):
**只 dispose buffer,descriptor 交给 GC。** 已抽成 `lib/editor/image_decode.dart:decodeCapped`,
配 `test/image_decode_test.dart` 三个测试钉死,其中一个**专门验证「提前 dispose 会坏」**——
防止将来有人当泄漏"顺手清理"掉。

---

## 6. 导出/导入:round-trip 是不变量

- **Mica 自己的导出没有单层顶壳** → 导入时不触发剥壳/造壳,原样还原。这是红线。
- **folder 的名字只在 manifest `title` 里**(它没有 `.md`/H1 可兜底),
  导入必须读 `title`,否则拿到的是路径里被 sanitize 过的名字(空格标点变 `_`)。
- **页面的名字是页面的属性,不是正文里的一行。** 导出不写 `# {name}`,导入不从正文提取。
  **Notion 是唯一例外**(它把标题同时塞进文件名和正文首个 H1),
  且只在精确匹配时剥离——一个恰好在开头的标题是作者的内容。
- **资产引用按 basename 兜底解析**,但**歧义时不猜**(两个 `logo.png` 就放弃),
  宁可留死链也不要接错文件。

---

## 7. 调研同类产品,专门用来证伪自己的前提

面对没有明显正确解的架构决策时,先去扒同类产品的**真实实现**。
重点不是"它支不支持",而是**"在和我们相同约束下它具体怎么做的、又刻意没用什么"**——
排除法往往信息量最大。

最该警惕的是自己脑子里"必须 X 才能 Y"那类前提。

**实例**:mermaid 桌面渲染曾基于"服务端渲 mermaid 必须 headless 浏览器"这个**错误前提**,
差点选 Kroki / Node / Chrome 那条路;扒了 AppFlowy + AFFiNE 之后才发现纯 Rust 渲染器
这条离线 + 跨平台 + 无浏览器的更优路径。

> 给出"几选一"之前,先自问:这些选项的**共同前提**验证过没有?

参照系:AppFlowy(Flutter 原生同构)、AFFiNE(web / Yjs 对照)。

---

## 8. 性能:先量,再改

- **MCP / REST 的天花板是网络 RTT,不是载荷。** 实测全端点 55–100ms,与载荷大小无关。
  写一篇 47 块的文档 = 一次往返 ~60ms。**问题从来在 token 和能力缺口,不在延迟**——
  别再优化"快"。
- **工具描述就是模型对工具的全部认知。** `search` 的描述曾写成 "by title",
  而实现一直是全文扫描 + 返回片段。一行错描述把已有的能力藏了整整几个版本。
- 编辑器侧真正的持续负载源(审计实锤):无界的预览栅格缓存、
  没有视口裁剪的 paint 循环、无上限的图片解码、setState 驱动的光标闪烁
  (会触发全文档 relayout)。这几处都已修,修法记在 git log 里。

---

## 9. 分层部署:哪些改动要重发什么

一个反复被忘记的事实:**服务端改动随 api 部署即生效;但 MCP 代理层的改动在
`mica-cli` 二进制里**——用户不把 MCP 指向新版 mica-cli 并重连,就还是旧行为。
排查"我明明改了怎么没生效"时先分清这一层。

---

## 10. 工具链与依赖的坑(重新发现要花几小时那种)

这些不涉及架构,纯粹是踩过一次就该记下来的。

### yrs

- **事务不可重入。** 持有 `transact()` 读事务时再调 `get_or_insert_map`,
  或者再开第二个 `transact()`,会**死锁**(不是报错,是卡住)。
  root 类型的句柄必须在事务**之前**取好;root id 在同一事务内内联读,
  别去调一个自己会另开事务的 helper。
- **`Attrs` 在 `yrs::types::Attrs`、`GetString` 在 `yrs::types::GetString`**,
  都不在 crate root。
- **`key: Null` 表示"这个格式已被清除"**,不是"没有这个键"。
  `marks_from_runs` 必须跳过 Null 值属性,否则 clear 之后 `to_blocks` 仍然带 bold。
  另外空 data 的表示是 JSON `null` 而不是 `{}`。

### flutter_rust_bridge / FFI

- ⚠️ **`frb integrate` 不加 `--no-write-lib` 会把真实的 `main.dart` 注释掉换成 demo**
  (留一行 `// temporarily commented out`)。接 frb 时务必带上这个 flag,
  出事了用 `git checkout` 恢复。
- FFI crate **自带 `[workspace]`**,防止被后端 workspace 收编。
- **rusqlite 钉 0.37**(libsqlite3-sys 0.35,落在后端 sqlx-sqlite 要求的 <0.38 内,
  共用同一个 native sqlite3,否则 link 冲突)。FFI crate 独立的 Cargo.lock 里
  dart-sys 锁死旧 `cc`,和 libsqlite3-sys 要的 `cc ^1.1.6` 冲突 →
  在 `clients/mica_flutter/rust` 跑 `cargo update -p cc` 同时满足两边。
- Windows 上 SQLite 打开的 db 文件被句柄占着,测试末尾 `deleteSync` 删不掉 →
  清理写成 best-effort try/catch(测的是持久化,不是清理)。

### SQL

- **`update` 是保留字**,列名用 `payload`。建表前先想一遍列名会不会撞。

### SVG / mermaid 渲染

- 纯 Dart 的 SVG 渲染器(flutter_svg、jovial_svg 都试过)**不解析 CSS 后代选择器**,
  所以 merman 的主题会全黑 → 自研 CSS 内联器把样式拍平进属性。
  merman 文档明确把 inline-styling 列为 host 边界,这不是 hack。
- flutter_svg 对 `font-weight: bolder/lighter` **直接抛错**(class 图会用到)。
- ⚠️ **内联器里 `rules.isEmpty` 不能提前 return。** class 图的 `<style>` 是空的、
  样式全部内联,但仍然需要走 font-weight 归一化。
  这个提前 return 卡了很久——**输出恒等,像极了 stale build**。

### 测试与 CI

- **多个后台 `cargo test` 会抢 build lock 卡住。** 一次跑一个。
- PowerShell 的 `Select-Object -Last` 会**缓冲到进程退出才输出**,
  看起来像卡死 → 一律 `cargo ... 2>&1 | Out-File log` 再 tail。
- **tag 触发的 workflow 用的是标签所在 commit 的版本。** 打 tag 的那个 commit
  必须已经含有该 workflow 文件;`workflow_dispatch` 也要等它进默认分支才在 UI 出现。
- **别拿"翻一个字节让 yrs 崩"当断言依据 —— `MicaDoc::from_blocks` 每次
  mint 随机 client_id。** 编码字节因此逐次不同,同一个偏移一会儿"过了
  decode_v1、在整合期 panic",一会儿"decode_v1 直接拒",断言具体错误变体
  就成了掷骰子。我写的 `a_corrupt_snapshot_is_an_error_not_a_panic` 因此在
  本地和一次 CI 绿、另一次 CI 红。**时绿时红比没有测试更糟:它训练人忽略红。**
  要确定性就 `from_blocks_with_client_id(..., Some(1))`;但实测钉住之后
  **再没有任何偏移能触发可捕获的整合期 panic**(172 字节逐偏移单进程扫过,
  非 decode_err 即 ok,字符串区还会撞上游 UTF-8 UB 直接 abort)。
  结论:**要测"守卫把 panic 收成错误",就直接喂一个会 panic 的闭包**
  (`contain_yrs_panic("d", || panic!(...))`),别绕道构造畸形字节;
  端到端那条只断言"返回 Err 而非 unwind"这个真正要守的契约。

### 用 computer-use 测桌面版

- `open_application "Mica"` 会解析到**已安装版**(`%LOCALAPPDATA%\Programs\mica`),
  不是 `flutter run` 出来的 debug exe——debug 路径不在授权列表里,截图会被打码。
  解法:`flutter build windows --release` 覆盖装到 installed 目录再测。
- PowerShell 的 `Set-Content -Encoding UTF8` 会给 `prefs.json` 加 **BOM**,
  Dart 的 `jsonDecode` 直接失败 → 设置静默落回默认值。用 `printf` 之类写无 BOM 的。
