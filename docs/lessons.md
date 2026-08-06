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

### 已收敛(2026-07-30,v0.13.4)

**第二套表示没有了。** 六步退役(S0 删死代码 → S1 补齐只走 op 的路径 → S2 存量回填 →
S3 协议版本闸门 → S4 停写 → S5 删表)做完,`document_snapshots` /
`document_updates` / `document_versions` 已从 schema 删除(migration 0016),
文档内容只存在于 `document_yrs_base`。

**这一节留着,因为它现在是"为什么不要再来一次"的证据。** 双表示活着的那几个月里,
它造出的不是一个 bug 而是一个**类**:导出空白、重开空白、MCP 写了没人看见、
`current_seq` 涨而内容不动。每一个都返回 ok,每一个都得靠用户报告才发现 ——
因为两份数据各自都自洽,不自洽的只有它们之间。

退役过程本身还留下两条可迁移的经验:

- **顺序不是洁癖,是回滚能力。** 先停写、再删表,中间隔一个版本才是稳的:删表之后
  回滚二进制,换来的是一台每次写入都 500 的服务器。这次是在知情下拍板一起发的 ——
  记下来,因为下次未必有人问这个问题。
- **否定式断言会无声腐烂。** 「不再写这两张表」这种断言,有人加回一条 INSERT 什么都
  不会红。S4 靠「写前写后各数一次行数」钉住它,S5 直接把它变成结构事实
  (`the_op_model_tables_are_gone` 查 `information_schema`)。同一个机制也是
  `docs/roadmap.md` 里那批「无 X」条目集体变假的原因。

仍然成立的一条:**恢复历史版本绝不能"把旧状态当 update 应用"**——CRDT 是并集,
不回退。必须用 `set_blocks` 在当前 doc 上重建目标内容。

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

### 端到端验证也会真空:跑的输入形态,真实部署根本产生不出来

2026-08-06,`JWT_SECRET` 自铸(v0.13.16)。提交信息里写了"端到端实测:生产模式、无
`JWT_SECRET`、全新库,起得来、自铸、重启后 token 仍然 200"。那次实测**是真跑的**,
结论也**是真的** —— 但它一行也没覆盖真实部署,因为**"没设"在容器里有两种形态**:

| 怎么来的 | 进程看到 | 走哪条分支 |
| --- | --- | --- |
| 手跑二进制,变量真的不存在 | `env::var` → `Err` | 自铸 ✅ |
| compose `${JWT_SECRET:-}`(示例文件也确实是 `JWT_SECRET=`) | `env::var` → `Ok("")` | 校验 → **拒绝启动** ❌ |

我手跑的是第一种,**每一个真实用户都是第二种**。整栈一起来立刻 crash-loop:
`JWT_SECRET is empty — an empty signing key signs anything`。

三条,按能复用的程度排:

1. **验证要跑"交付物产生的那个输入",不是"我手里方便造的那个输入"。** 差别就藏在
   `Err` 和 `Ok("")` 之间,读代码时它俩长得像同一件事。
2. **空白必须等同于未提供**(`resolve_jwt_secret`)。凡是经 compose/env 文件传的可选变量都适用 ——
   `${VAR:-}` 永远把未设解析成空串,`FOO=` 也是空串,没有哪一层会替你把它变回"未设"。
3. **这个 bug 是本地起整栈才抓到的,不是测试抓到的。** 当时那套 CI 也抓不到:
   `e2e/.env.ci` 把 `JWT_SECRET` 显式设成了一个 48 字符的值,于是 CI **只走
   operator-supplies-it 那条**,恰好是新版里更罕见的一条。**给了默认/自动值之后,
   要回头看 CI 是不是还在喂显式值** —— 喂着,新路径就一次也没被走过。
   (已删,CI 现在按文档的快速上手路径启动。)

4. **健康检查全绿 ≠ 功能可用。** 同一次实测里还撞出第二件事:`mica` 这个 bucket
   **没有任何东西会创建**(RustFS 是文件系统后端,bucket 就是 `/data` 下的目录)。
   四个容器全 healthy、`/api/ready` 200,**每次上传 404** —— 直到有人粘一张图才发现。
   建桶说明当时只有一句,埋在 `docs/deploy.md` 的 **Traefik 章节**里,看快速上手的人
   看不到。是"真的 PUT 一个文件上去"抓到的,不是任何 health probe 抓到的。
   **给关键路径写检查时,问的应该是"这个功能能不能用",不是"进程还在不在"。**
   〔修法本身也有个后续:第一版是 compose 里加 `rustfs-init` 跑 `mkdir`,当天就被指出**焊死了
   RustFS 的实现细节**(假设 bucket = 目录、假设镜像有 `rustfs` 用户),换 MinIO/OSS/S3 就废。
   改成代码里走 S3 接口探测+创建 —— 见 `docs/bucket-provisioning-plan.md`。教训是:
   **"能跑"和"没把某个实现焊死"是两件事**,前者当天就验证了,后者要有人问"换一个会怎样"。〕

〔另一半在 CLAUDE.md「部署零凭据可起」条。〕

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

### 断言"为空"的测试,可能是在测一个空数据

S5 换掉测试夹具的存储时挖出来的:`a_self_link_is_not_a_backlink` 断言「一个链到自己
的页面**没有**反链」—— 绿的。但夹具写的 link mark 是
`{"type":"link","href":"mica://page/<id>"}`,**没有 `start`/`end`**。旧的 op 快照把
`data` 当 jsonb 逐字存取,所以这个残缺的 mark 照样能被反链扫描看见;换到 yrs 之后
marks 存成文本 format,`marks_from_data` 会丢弃无区间的条目 —— 于是页面里根本没有
链接,「没有反链」这句话变得**恒真**。

同一份夹具喂给另外两个断言「**有** 1 条反链」的测试,它们立刻变红,这才暴露出来。
换句话说:**是正向断言把反向断言的空转揪出来的**。

一般化,比"要写负面测试"更有用的那条:
> **一个只断言"没有 X"的测试,必须有一个共享同一份 fixture、断言"有 X"的兄弟。**
> 否则 fixture 一旦悄悄变空,前者永远绿,而你以为自己在守一条规则。

也是「双表示」的余震:两套存储对**同一份残缺输入**的宽容度不同,所以夹具会依赖上
其中一套的宽容而不自知 —— 迁移存储时这类依赖才会一起浮上来。

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

### web 上「应用完全没起来」可以是**零错误输出**的

2026-07-30 新建 web e2e,第一次真跑就抓到:headless Chromium(未指定 locale)下页面加载后
抛 `Error: Invalid argument(s): Incorrect locale information provided`,发生在**引擎层的
locale 探测、`runZonedGuarded` 之前**。于是 `main()` 走不到 `registerYjsSelfTest()`,
`window.micaYjs*` 全 undefined,`document.body.children` 只剩 1 个(canvas 外壳)——
**整个应用没起来**。

而当时 **console 里一条错误都没有**:

> **未捕获异常不是 console message。** Playwright 里 `page.on('console')` 收不到它,
> 要 `page.on('pageerror')`。只监听 console 的 web 检查,对「启动就抛」这一整类是瞎的。

两条推论:
- `main()` 里那一串启动步骤是**串行且脆弱**的:任何一步抛,后面全部不执行。本仓库的
  `registerYjsSelfTest()` 排在 `loadPersistedLocale()` 之后,所以一个 locale 问题连带
  让**测试钩子也消失**——症状于是变成「探针不存在」,离真因很远。
- 上游/引擎层的错误逃出你自己的 zone 是**正常的**,`runZonedGuarded` 不是全能网。
  它捕不到的东西,只有浏览器级的 `pageerror` 能看见。

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

#### 「本地跑不了」不等于「本地验不了」—— 别拿 CI 当编译器

新增 `web-e2e` job 那次**连红四轮,四个根因全是我的**,而且**前三个本机都能验**:

1. 照抄了 `rust` job 的 psql 迁移循环 → 那步只建 schema、**不写 `_sqlx_migrations`**,
   api 启动时 `run_migrations` 于是从头重放,撞 `relation "users" already exists`。
   (`rust` job 没事,因为 `cargo test` 从不调 `run_migrations`。)
2. 就绪等待写成 `curl … && break`,配 `set -euo pipefail` + `shell: bash -e` →
   **失败的 curl 把整个脚本带崩,「轮询 60 次」实际只跑了一轮**。日志里 api 的 "ok"
   出现在错误的下一行 —— 它只是还没起来。
3. `nginx.dev.conf` 挂到了 `conf.d/default.conf` → 它是**整份** nginx.conf(开头就是
   `worker_processes 1;`),nginx 拒绝启动;而我的轮询把这件事报成「nginx 连不上 api」,
   误导六十秒。**而 `docker-compose.yml` 里一直写着正确的挂载点,我只是没照抄。**
4. headless Chromium 的 locale 让应用启动即崩 —— 这一个确实只有真跑才知道。

能在本地验的部分:

```sh
# 挂载方式与 job 完全一致,验配置本身
docker run --rm -v "$PWD/deploy/nginx.dev.conf:/etc/nginx/nginx.conf:ro" nginx:alpine nginx -t
# 步骤脚本的 shell 语义(尤其 set -e 与 && 的相互作用)
bash -n <<<'<该步骤 run 的内容>'
# YAML 用拒绝重复键的 loader —— safe_load 漏重复键而 GHA 严格拒绝
```

第三轮之后我才改成先在本机 `nginx -t`,一次通过。

一般化,而且比「多测测」更可执行:
> 改 CI 时,先把每一步**能在本机复现的那部分**单独跑一遍:配置文件语法、挂载路径、
> 步骤脚本的 shell 语义。真正只有 CI 能暴露的通常只有**环境**(runner 的 locale、网络、
> 预装工具),那一类才值得用一次 push 去换。

**推论:让 harness 会解释自己。** 第四轮之前脚本超时后**一个字的诊断都没打**,下一次尝试
只能靠猜。加上「超时就把 url / title / `window` 上各钩子的 typeof / pageerror / console /
失败请求 / 请求过的 host 全部打出来」之后,**下一轮 CI 直接给出了根因**。一次诊断改动省掉
的往返,比它自己贵得多。

### 用 computer-use 测桌面版

- `open_application "Mica"` 会解析到**已安装版**(`%LOCALAPPDATA%\Programs\mica`),
  不是 `flutter run` 出来的 debug exe——debug 路径不在授权列表里,截图会被打码。
  解法:`flutter build windows --release` 覆盖装到 installed 目录再测。
- PowerShell 的 `Set-Content -Encoding UTF8` 会给 `prefs.json` 加 **BOM**,
  Dart 的 `jsonDecode` 直接失败 → 设置静默落回默认值。用 `printf` 之类写无 BOM 的。

### CI 缓存:先量命中率,别信"加了缓存就快"

- **`sccache` 加在 tag 触发的 release job 上 = 命中率恒 0%,纯开销。** GitHub
  Actions 的 cache **按 git-ref 隔离**:release workflow 只在 `refs/tags/v*` 上跑,
  每个 tag 是独立 ref,存的缓存另一个 tag 读不到(只能读默认分支 main 的缓存,而
  release job 从不在 main 上跑 → main 永远没有这份缓存)。实测 v0.12.18(理应"热")
  windows job:`Compile requests 413 / Cache hits 0 / Cache misses 326 / hit rate 0.00%`。
  加了三次发版(0.12.16 无→685s、0.12.17 冷→664s、0.12.18 "热"→755s),热那次反而最慢。
- **就算命中也救不了:量 job 里到底谁慢。** windows job 的长杆是
  `flutter build windows`(495s / 66%),那是 Dart/C++,`sccache`(只包 rustc)碰不到;
  能缓存的 Rust FFI 只是一小片。**唯一像样的未缓存 Rust 是 `images(api)` 的 Docker 编译
  (415s),但它和 windows 并行、不在关键路径,省了也不减发版墙钟。**
- 结论:**缓存要落在"频繁 + 在关键路径 + 缓存能持久"的地方。** CI-on-main 已挂
  `Swatinem/rust-cache` 且总耗时才 ~3min(main 分支缓存能持久),那才是对的。发版慢的
  真杠杆是 Flutter 构建(更大 runner 或接受它),不是 Rust 缓存。v0.12.18 撤掉 sccache。

### 本地 dev 栈:容器里的 API 做不了「服务端直传」〔2026-08-06 已修,但留着看根因〕

> **已修**:`S3_INTERNAL_ENDPOINT`(dev 栈设 `http://rustfs:9000`)。服务端自己发的 PUT
> (`presign_put_server`)签这个地址,交给浏览器的(`presign_put`)仍签公网地址。下面的绕法
> **不再需要**,但根因留着 —— 它咬了很久,而且**咬第二次的时候我没认出来**:
> 2026-08-06 做建桶时"发现"服务端够不着 S3,当成新问题写进了提交信息,其实这一条早就在这儿。
> **教训是关于这份文档的用法**:写下来不等于会被读到,动到同一块地方时得先搜一遍。
> 〔另一半:改的时候只顺手修了 `blob_gc` 和桶级操作两处,**漏了本条早就列全的另外三处**
> (换头像 / `import_url` / `store_bytes`)—— 清单就在眼前也会漏,除非按清单逐条核对。〕

- `docker-compose.yml` 的 `S3_ENDPOINT: http://127.0.0.1:9000` 是**给浏览器用的**——
  后端只负责*签名*,presigned URL 的 host 必须是浏览器能到的地址,不能写 compose 服务名。
  代价是:**任何由服务端自己发 PUT 的路径在容器里必然失败**,因为容器内 `127.0.0.1`
  是它自己。踩到的是换头像(`PUT /api/auth/me/avatar`),同理还有 `files::import_url`
  和导入时重托外链图片(`store_bytes`)。报错长这样:
  `storage upload failed: error sending request for url (http://127.0.0.1:9000/...)`。
- **绕法:API 跑在宿主机上**,`docker stop mica-api` 腾出 8080,然后用 compose 里同一套
  环境变量 `cargo run -p mica-api-server`(DATABASE_URL 指 `127.0.0.1:5432`,S3 指
  `127.0.0.1:9000`,两个端口都已 publish)。这样浏览器和服务端解析到同一个地方,
  `deploy/nginx.dev.conf` 本来就把 `/api/` 代到 `host.docker.internal:8080`,web 端
  (:8090)不用改。生产没这个问题:那边 `S3_ENDPOINT` 是真正可达的对象存储地址。

### Route 看不到父级的重建(改完东西,你操作的那个面板是最后知道的)

- 设置对话框是 `showDialog` 的 route。父级 `setState` 会重建侧边栏,**不会**重建它;
  而 `await 动作()` 刚回来时,子 State 的 `widget.xxx` 还是旧值(父级的重建要等下一帧)。
  实测症状:换头像后侧边栏头像变了,你点「更换头像」的那个面板还画着旧的首字母。
- **修法不是等一帧**(`endOfFrame` 那类时序 hack),而是让动作**返回动作之后的状态**:
  `Future<String?> onChangeAvatar()` 回新的头像 URL(用户取消就回原样),面板拿返回值
  存进自己的 state。route 从此不依赖任何重建。
- 附带教训:这类 bug 单测抓不到,因为 `_SettingsDialog` 是 `part of main.dart` 的私有类
  ——`part of` 反模式的又一次收费。

### 内容哈希做缓存键:负结果也会被缓存

- 头像 URL 用内容哈希当 `?v=`(同一张图不换 URL,省一次下载)。副作用:
  **装 → 移除 → 再装同一张**会回到那个中间 404 过的 URL,而 Flutter 的 `ImageCache`
  把那次失败留到进程结束 → 应用说你有头像,画的是字母。
- 修在唯一知道「这个 URL 背后的字节刚变了」的地方(写头像那一步)`NetworkImage(url).evict()`。
  一般化:**只要缓存键是内容派生的,就要想清楚"同样的内容再次出现"这条路——尤其中间
  夹过一次失败。**

### 对话框的 TextEditingController 该由路由持有,不是调用方

- 四个对话框都是这个写法:`final c = TextEditingController();` → `await showDialog(...)`
  → `c.dispose();`。**那个 future 在 `Navigator.pop` 时就完成,退场动画还让 TextField
  挂着一两帧**;这期间任何一次重建都会让 `InputDecorator` 的动画去重新监听一个已 dispose
  的 notifier → `A TextEditingController was used after being disposed`,紧接着
  `_dependents.isEmpty` 红屏。登录和添加服务器都是 pop 之后立刻 setState(整棵树重建,
  含正在退场的路由),所以**必崩,不是竞态**。「添加服务器」输入框为空时不崩,是因为那条
  路径在 setState 之前就 return 了 —— 这就是它看着像偶发的原因。
- 修法是所有权,不是时序:controller 放进路由自己的子树(`ui/dialog_controllers.dart`
  的 `DialogTextControllers`),`State.dispose` 在路由真正消失时才跑。别去猜动画多长
  (`addPostFrameCallback` 那类都不可靠)。
- **这类只有真跑桌面端才撞得到**:单测不会在退场动画中途重建,web 端也没走这条路。

### 报告写进了用户没在看的那块 UI = 没报告

- 「清理缓存」把「已释放 X」写进了 `_accountMsg`,而那个变量只在设置的**账户**区渲染,
  按钮在**数据**区 —— 点完按钮,数字变了、没有一句话说为什么。眼看才发现,`flutter test`
  和 curl 都测不出来。
- 一并的教训:文案要说清**动作**,不只是数字。镜像里没有图片时「已释放 0 B」读起来像什么
  都没干,而页面确实清了 → 「已清理云端镜像,释放 {size}」。

### 只改了一个平台变体,分析器不会告诉你另一个断了

- 2026-08-02:给本地世界加了 `exportDocMarkdownText`,只写进 `local_offline_io.dart`。
  `flutter analyze lib` 一路绿、`flutter test` 1034 全过、桌面跑得好好的 —— 因为
  **分析器只解析条件导入里当前平台那一个变体**。web 编译解析 `local_offline_web.dart`,
  当场 `The method 'exportDocMarkdownText' isn't defined`。CI 连红五次。
- `lessons.md` 里原本记的是「web 通过≠桌面通过」。这次是**它的反面**,而且更隐蔽:
  桌面侧的所有本地检查都不可能发现,不是"忘了测",是**结构性看不见**。
- 规则:**动了 `clients/mica_flutter/lib/local/` 或任何 `_io`/`_web`/`_stub` 变体,
  提交前必须跑一次**
  `flutter build web --release --no-web-resources-cdn`(本机约 110s)。
- 三份要一起改:`local_offline_api.dart` 声明、`_io` 真实现、`_web` 返回 null。漏掉接口
  那份的话,两个实现会各自漂移而没人拦。

### `cargo check` 不跑 clippy,而 CI 是 `-D warnings`

- 同一天同一批提交:写完 Rust 只跑了 `cargo check`,绿。CI 的
  `cargo clippy --workspace --all-targets -- -D warnings` 挂在一个多余的 `mut` 上。
- 一个 `mut` 让整条流水线红了五次 —— 因为 `-D warnings` 把**任何** clippy 提示变成错误,
  而 `check` 一条都不报。
- 规则:提交 Rust 前跑 **CI 的原样命令**,不是 `cargo check`,也不是不带 `-D warnings`
  的 clippy(那样只会打印告警然后退出 0,看着像过了)。
- 更一般的一条,这两条都是它的实例:**本地跑的命令和 CI 跑的命令不一样时,"本地绿"
  什么都不证明。** 要么跑同一条,要么别声称验证过。
