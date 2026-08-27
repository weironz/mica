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

### 纯函数全绿,而调用它的那一行是错的

2026-08-27,侧栏 Ctrl/Shift 多选。规则里所有**能算错**的部分都抽成了纯函数,配 41 条
测试,全绿。装配好的 app 里它是坏的:Ctrl 点第二行,选区又回到一行。

```dart
_selectedViewIds..clear()..addAll(selectionAfterToggle(_selectedViewIds, id))
```

Dart 级联的**参数在 `clear()` 之后求值**,于是 `selectionAfterToggle` 每次读到的都是空集。
函数是对的,它的每条测试也是对的,错的是它上面那一行 —— 而那一行住在 State 里,
**测试够不着的地方**。

真正值得记的是第二步,因为我差点就那样交付了:我写了一条 widget 测试"覆盖"它,
然后**把 bug 原样放回去验有没有牙 —— 测试照过**。它护的是我在测试里自己写的那份
驱动逻辑,不是生产代码。一条测不到生产代码的回归测试,比没有更坏:它让人不再去看。

修法不是把那行拆成两句(那样 bug 仍然活在够不着的地方),是把**有状态的累积搬出
shell** —— `TreeSelection` 持有 ids + 锚点,13 条测试直接打在它上面,放回级联时 7 条
当场变红。

两条:

1. **"把能算错的抽成纯函数"只走了一半。** 纯函数保证不了**有没有人正确地调用它**。
   状态转移(累积、清空、剪枝)本身就是能算错的部分,它也得搬到测试够得着的地方。
2. **验证一条回归测试有没有牙,要把 bug 放回它防守的那个位置**,不是放回一个长得像的
   位置。放错位置的"有牙"验证,和没验证是一样的,只是更让人放心。

### 自动化测试也会造假阴性,而假阴性比假绿更耗人

这一节前面全在讲「测试真空通过」——**报了绿,其实没验**。2026-08-27 撞上的是它的反面,
一样危险:**报了红,而功能是好的**。

侧栏多选做完后,我用 Playwright 在 web 上验 Ctrl/Shift 多选。结果:Shift 一次没成功过,
Ctrl 时好时坏。我据此认定 web 上有 bug,**连烧五次 web 构建**去追它 —— 换 `isShiftPressed`、
加 `debugPrint`、换 `print`、加 DOM 监听、比对 Ctrl 与 Shift。最后是**用户一句「我手动测
shift 是可以正常选的」推翻的**。

排查过程里有两个事实,当时都没被我用对:

- DOM 层监听到的 `pointerdown` **两次都带着正确的 `shiftKey` / `ctrlKey`**。我当时读成
  「浏览器没问题,所以是 app 的问题」—— 但它其实说明的是:事件**送到了浏览器,没送进
  Flutter 的 `HardwareKeyboard`**。中间还隔着一层,而那一层正是 CDP 合成事件的薄弱处。
- 编辑器里 `Shift + 方向键`(走同一个 `isShiftPressed`)**在 Playwright 下是好的**。
  我把它读成「Flutter 能看见 Shift,所以侧栏代码有问题」—— 正确的读法是
  **「Shift + 按键」可靠,而「Shift + 点击」不可靠**,两条路径在 Flutter web 里不是一回事。

三条:

1. **合成输入(CDP / Playwright)测不了 Flutter web 的「修饰键 + 点击」。** 别再用它下这类
   结论,不管结论是红还是绿 —— 我早先那句「Ctrl 在 web 上验过了」同样不算数,那是碰巧。
2. **一个红结果在追第二轮之前,先问「会不会是测具坏了」。** 判据现成:用**同一套测具**去跑
   一件**已知一定成立**的事(这里就是编辑器的 Shift+方向键,或桌面端同一手势)。这一步
   五分钟,能省掉五次构建。
3. **要让一个功能可自动化验,得给它一条不依赖脆弱输入的入口。** AFFiNE 的多选走行首复选框,
   点一下就是选中 —— 那种交互 Playwright 测得了。这是产品决定,不是测试决定,但代价要算进去。

### 快捷键可以从来就没生效过,而且不报任何错

同一次里的第三件事。我给多选加了「Esc 清空选区」,写进了设置面板和 `shortcuts.md`。
它一次也没工作过,而且**没有任何失败信号** —— 按下去什么都不发生,和"没绑"长得一样。

三种绑法全试过,全是死的:树本地的 `Shortcuts`/`Actions`、给树加 `FocusNode` 后再绑、
以及 `_appShortcuts`(`Ctrl+N` 就在那儿而且能用)。探针给出的事实是:选中操作后 400ms
`primaryFocus` 确实是侧栏,但**按下 Esc 时焦点已经回到页面路由自己的 scope** —— 那是
整个 shell 的祖先。Flutter 从焦点节点**向上**分发,所以 shell 内部注册的任何快捷键都
看不到这个键。

两条:

1. **加了快捷键必须真按一次。** 它是少数几种"写完看起来完全正确、编译通过、测试通过、
   而运行时是空操作"的改动 —— widget 测试也看不见,因为测试里焦点是你自己摆的。
2. **按不动的快捷键要撤掉,不是留着。** 文档里写了一个按不动的键,比不写更坏。要做只能
   上全局 `HardwareKeyboard` 处理器,而它会在对话框/行内改名正拿着 Esc 时一起触发 ——
   那是另一个决定,不是顺手补全。

### "为什么做不了"的结论也会烂,而且没人去核它

2026-08-05 撤回重连合并时,回退提交把根因查得很准(ack 只沿逐条 ack 的连续前缀推进),
然后写下一句结论:**「前置是改 ack 语义,那是协议改动,不是优化」**。这句进了 roadmap,
把这条挡了整整一个版本。

2026-08-06 真去做的时候,第一步是读服务端怎么用那个 id —— `ws.rs` 拿 `envelope.id`
**原样回显**,从不解释它。所以「id N = N 及以前全部落库」这个语义**完全活在客户端**,
`WS_PROTOCOL_VERSION` 一动没动。缺的从来不是协议,是客户端不知道每次推送覆盖了哪段。

**两次错在同一个地方,方向相反**:
- 做的时候写「协议不用动」,**没读 ack 推进逻辑** → 上线即卡死;
- 撤的时候写「那是协议改动」,**没读服务端怎么用 id** → 一个版本没人碰。

**乐观的假设会被 CI 抓到,悲观的假设不会。** 前者产出一个红测试,后者产出一句"暂时不做",
然后安静地留在清单上 —— 没有任何机制会去证伪它。所以:**「因为 X 所以做不了」这类结论,
要和"做完了"一样被当成待核事实**,尤其当它是自己写下的、且刚吃过一次亏的时候
(那时候人最容易把代价估高)。CLAUDE.md 第 6 条说的「先问这些选项的共同前提验证过没有」,
指的就是这个。

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

### 「重建了却没生效」:先证明浏览器跑的是哪份包

2026-08-15 验证上传超限文案时,浏览器里红条一直显示服务端的英文原文,不是新加的中文。
干净重建(`rm -rf build/web`)后重测,**一模一样**;确认打包产物里两句新文案都在;
排除了 service worker(0 注册)。于是我报告「客户端那半没生效」——**结论是错的**。

真因是 **HTTP 缓存**:`python -m http.server` 不发 `Cache-Control`/`ETag`,Chrome 就按
`Last-Modified` 做启发式缓存,在每隔几分钟的重建之间一直复用旧的 `main.dart.js`。
而用来翻页的 `?v=xxx` **只能让 `index.html` 失效** —— `main.dart.js` 是固定 URL,照旧走缓存。
那几轮「重建后重测」跑的其实是同一份旧包。

最坑的是它**自我强化**:插进代码里的探针也在没被加载的那份包里,于是探针不显形,
而「探针没输出」被读成了「这段代码没执行」,顺着这个错误前提又推了两轮。
这和上一节的 `registerYjsSelfTest()` 是同一种病 —— **症状是「探针不存在」,真因在别处**。

对策(按代价排):

- 起静态服务就带 `Cache-Control: no-store`,别用裸 `python -m http.server`。
- 怀疑代码之前,先花一行**证明包被加载了**:
  `await (await fetch('/main.dart.js')).text()` 里 grep 一个只有新版才有的字符串。
  这一步比读代码便宜得多,却能一次排掉「陈旧」这整类原因。
- 查缓存要查**三层**,只查一层等于没查:service worker、HTTP 缓存、磁盘上的产物。
  当时只查了第一层和第三层,漏掉的正是中间那层。

顺带,一个反复挡路的组合:**web 的 token 是内存态**(`prefs_web.dart`,有意为之),
所以每次 reload 都掉登录;而登录表单只能靠无障碍树驱动,一开无障碍焦点又被
`FLT-SEMANTICS` 抢走、自绘编辑器的 IME 收不到按键。两个需求互斥,自动化会卡死。
绕法:**把视口缩到表单居中**(460×714),`flutter-view` 的中心点就落在输入框上,
真键盘直接可用,全程不碰无障碍。

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
- **页面的名字是页面的属性,不是正文里的一行 —— 但这个属性住在文档里。**
  〔2026-08-27 改写。原文是「导出不写 `# {name}`,导入不从正文提取」,已被
  `page-title-plan.md` 推翻:名字曾经只在 `views.name` 列上,于是文本一离开 Mica
  就丢了。现在标题在文档的 root 块(`root.data['title']`),导出**写** `# {name}`,
  导入把它**剥回**属性。「不是正文里的一行」这条继续成立:root 块不是正文块。〕
  剥离只对**自家归档**(manifest `generator: mica`)和 **Notion**(它把标题同时塞进
  文件名和正文首个 H1)生效,且**只在精确匹配时** —— 一个恰好在开头的标题是作者的内容。
  两侧(`with_page_title` / `strip_leading_h1`)**必须一起改**;`interchange` 里那条
  `a_mica_export_survives_import_and_re_export_byte_for_byte` 是唯一同时走过两边的测试。
- **一条规则被推翻时,要搜的是它的每一处表述,不是改掉最先看到的那处。** 这一条自己就是
  证据:P1 改了 `export-import.md` 第 67 行、漏了同一份文档第 130 行的整段,也漏了上面
  这条 —— 三处说的是同一件事,后两处到 P3 才补上,中间隔了一整轮实施。
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

### 缓存的**跳过条件**是一条不变量,新增写入者必须一起维护

2026-08-28 发现的一个缺陷(v0.13.31 引入)。**它是否真的咬到过人,至今没有证实** ——
用户那台 0.13.31 上出现过一次 `not found`,条件全部对得上(版本对、origin 正常、
点其它页面都能打开、只有自动打开的那一页 404),但没等取到判据症状就自己消失了。
它本来就是间歇的:任何一次拿到 200 的完整拉取都会把状态自愈,下次再犯。
**「症状消失」不是「原因查明」**,所以这里写「对得上」而不是「就是它」。

〔**这条的来历本身就是一个教训,所以原样留着改**:它是在排查用户报的 `not found` 时读代码
发现的,我当场把它写成了「用户从第二台机器报回来的就是它」。后来查装机版本 —— 那台是
**0.13.28**,根本没有这段代码 —— 症状另有原因(见下一条:我的 dev 调试改坏了共用的 prefs)。
**读代码发现的真缺陷,和眼前那个症状,是两件事**;把前者当成后者的解释,是在没有证据的地方
造了一条因果。缺陷是真的、修法是对的,但它当时并没有伤到谁。〕

工作区加载路径只在 **200** 时写磁盘镜像,304 时跳过,注释写着理由:
「a 304 means the mirror ALREADY holds this tree」——**这是一条不变量**,不是一个优化开关:
*保存的 ETag 意味着镜像已经持有那棵树*。

当天新加的实时页面树刷新(`_refreshTreeFromServer`)拉到新树后**保存了新 ETag、更新了内存,
却没有写镜像**。于是 ETag 跑到了镜像前面。而且**当场什么都不会发生** —— 症状要等到
**下一次冷启动**:预热旧镜像 → 带新 ETag 条件请求 → 服务端 304 → 用旧树 →
自动打开一个服务端已经没有的页面 → bootstrap 404。

**三条**:

- **「跳过写入」的条件就是不变量本身。** 加一个新的读取/刷新路径时,要问的不是「我需不需要
  写缓存」,而是「有没有别的代码**因为我保存了这个值**而选择不做某件事」。
- **打破它的代价延后一个进程生命周期才显形。** 我在同一台机器上端到端验过这个功能(建页/
  删页、真机侧栏自己长自己缩),全绿 —— 因为破坏只在**下次启动**才可见,而我从没重启过它。
  **验收要包含一次冷启动**,尤其是动了任何持久化的东西。
- **别把「读代码发现的缺陷」当成「眼前症状的解释」。** 两者都真,但它们之间的因果需要证据。
  一条一刀切开的判据往往很便宜 —— 这次是「装的是哪一版」:0.13.28 里根本没有这段代码,
  一查就知道它不可能是原因。**先找那个判据,再讲故事。**

### `flutter run` 和用户装的正式版**共用一份 prefs**,dev 调试会改坏它

2026-08-28:为了验证一个窗口 bug,我用
`flutter run --dart-define=MICA_API_BASE_URL=http://127.0.0.1:8080` 跑 dev 版。
它启动时把 `cloudOrigin` / `activeOrigin` / `servers` **写成了本地地址** —— 而
`%APPDATA%/mica/prefs.json` 是**和已安装的正式版同一个文件**。用户下次打开正式版,
它拿着真实云账号的身份去问本地服务器要工作区,侧栏弹出红色 **not found**。
用户以为自己的应用坏了,实际是我的调试留下的。

`main.dart` 里那段注释**已经预见了一半**:「a dev build inherits whichever server the
installed copy … one keystroke away from writing test data into real notes」——所以
`MICA_API_BASE_URL` 才优先于存档值。**防住了读的方向,没防住写回去**:pinned 值照样被
持久化,污染是反方向发生的。

**规矩**:在用户真实机器上 `flutter run`,**给它一个隔离的配置目录**,别写共用的那份 ——

```powershell
$env:APPDATA = "<scratch>"; flutter run -d windows
```

桌面端 `configDir()` 读的就是 `APPDATA`(Windows)/`HOME`(mac、Linux),换掉即可完全隔离:
dev 版拿到干净的 prefs(自己登录本地栈),用户的会话、服务器、窗口几何一律不动。
**代价是零**,不这么做的代价是用户以为自己的数据出了问题。

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
- **2026-08-26,同一个形状,这次在安装脚本上**:`install.ps1` 很早就修过「写不进正在运行
  的二进制」——它的注释写得清清楚楚,连为什么 `.old` 必须用唯一名都记了。而 `install.sh`
  从来没跟上,一直是 `curl -fSL "$url" -o "$dest"`,也就是**原地截断**。Linux 对正在执行
  的映像拒绝这种写入(`ETXTBSY`),所以「重跑一行就能更新」在 **MCP 客户端开着时必然
  失败** —— 正是最需要它工作的那批人。实测(容器内,用 fifo 把 `mica-cli mcp` 真的挂住):
  同一时刻原地写 `refused (ETXTBSY)`,改成「下载到旁边再 rename」后 `OK`,且原进程继续活着。
- 规则补一条:**一个坑修在某个平台变体上时,当场去看它的兄弟变体**。这两次都不是「忘了
  测」,是修的人当时手上只有一个平台;而且修复的注释写得越好,越容易让人以为这件事整体
  已经解决了。

### `cargo check` 不跑 clippy,而 CI 是 `-D warnings`

- 同一天同一批提交:写完 Rust 只跑了 `cargo check`,绿。CI 的
  `cargo clippy --workspace --all-targets -- -D warnings` 挂在一个多余的 `mut` 上。
- 一个 `mut` 让整条流水线红了五次 —— 因为 `-D warnings` 把**任何** clippy 提示变成错误,
  而 `check` 一条都不报。
- 规则:提交 Rust 前跑 **CI 的原样命令**,不是 `cargo check`,也不是不带 `-D warnings`
  的 clippy(那样只会打印告警然后退出 0,看着像过了)。
- 更一般的一条,这两条都是它的实例:**本地跑的命令和 CI 跑的命令不一样时,"本地绿"
  什么都不证明。** 要么跑同一条,要么别声称验证过。

### 空转的 sweep 打印 "Done: 0 re-hosted, 0 failed."，被当成"没有可做的"

- `mica-cli rehost-images` 判断块类型读的是 `block["kind"]`，而 `/bootstrap` 的载荷里
  这个字段叫 **`type`**。于是它对**每一篇文档**都不匹配，全库扫完输出
  `Done: 0 re-hosted, 0 failed.` —— 退出码 0，措辞像"检查过了，没有要做的"。
- 实际情况是库里有 **917 个外链图片引用**（其中 289 个 appflowy）。修好字段名后同一条命令
  一次跑完收编了 **488 张**。这个空转带着"成功"的外观活过了一个发版。
- **危险的不是它不工作，是它汇报成功。**报错会被追查，`0 failed` 不会。凡是"扫描 N 个东西
  然后汇报"的命令，**0 命中和 0 存在必须能区分** —— 至少把扫描到的候选总数一并打印出来
  （"检查了 15172 篇文档、37 个图片块"），否则"没找到"和"没在找"长得一模一样。
- 配套的第二条：这类 sweep 用 `?` 直接把单次网络错误往外抛，一个瞬时
  `os error 10054` 就终止整轮。逐项失败该计数继续，只有认证/配置错误才值得中止。

### 埋点的第一次真机运行，抓到的是加埋点那个人自己的回归

- 工作区切换优化分三阶段：第一阶段加了 `SwitchTrace`（把一次切换拆成
  shell / mirror / tree / body 并附每个请求耗时），第二阶段让正文先由设备上的镜像
  铺底。第二阶段我把 `_seedBodyFromMirror` 写成了 `await`，串在页树请求**之前**。
- 理由当时听起来无懈可击：**"只是本地读一下"**。实测：大文档上 **836ms**——解码一份
  镜像文档是实打实的活。于是热路径 **1054–1563ms**，比冷路径（109ms）慢十倍。
- 改成与网络并发后：**67–273ms**，`body` 稳定 2ms。
- 两条规则：
  1. **优化之前先让它可测量。** 前一轮的"优化"是照着 curl 从宿主机量的数发的，
     方向对（成员移出关键路径、bootstrap 并发）但没人知道剩下的时间花在哪；
     真正的大头（一次 TLS 握手 70–110ms、以及后来这个 836ms）都不在那份读数里。
  2. **"只是本地读一下"不是免检理由。** 本地 ≠ 便宜。凡是要 `await` 的东西，先问
     它凭什么排在用户看得见的东西前面。

### dev 构建 ≠ dev 数据

- 桌面开发版默认连 `http://127.0.0.1:8080`（`_resolveBaseUri`），但它**先读
  `%APPDATA%` 里保存的服务器地址**。于是 `flutter run` 起来的第一件事是登录到
  **生产**（界面直接显示 344 页的真实工作区）。
- 那一轮原计划要做的是"丢字红线"测试：打字 → 切走 → 切回 → 看字还在不在。**照那样跑，
  写的是用户的真实笔记。**
- 规则：**跑任何带写入的验证前，先确认连的是哪个后端**；桌面端用独立 `APPDATA`
  （`local_offline_io._localDir()` 由它派生）起一个干净档案，两个实例就什么都不共享
  ——store.db、blob、prefs 全部隔离。顺带解决单实例互斥锁那个"验证要先关掉用户的
  应用"的死结。

### 空字符串又一次冒充「未设置」——这次在客户端

- 干净档案里 `cloudOrigin` 是**空字符串**（不是缺失）。`Uri.parse('')` 的 host 也是空，
  于是 `_isLocalBackend()` 判假 → dev 自动登录被跳过 → 新装的开发者看到的是登录页，
  而不是文档承诺的"自动登录"。
- 和部署那次是同一个形状：compose 的 `${VAR:-}` 把未设变量解析成空串，`env::var`
  因此返回 `Ok("")` 而不是 `Err`，JWT 自铸功能整个失效（见本文件相应条目）。
- 还有第二层：自动登录**成功**之后也不写 `cloudOrigin`，而"切换世界"页是按已配置
  服务器列举的——于是那个会话在界面上是孤儿，云端世界根本进不去。
- 规则：**任何"有没有配"的判断，空白必须等同于未提供**；而且判断"配了"之后，要检查
  配置是否真的写进了消费它的那个地方。

### Range 请求会把 404 错误页伪装成「200 系」

- 清理死图前要判断"这张图还在不在"，探测用的是
  `curl -o /dev/null -r 0-1023 -w "%{http_code}"` —— 加 Range 是为了不下整图。
- 结果：一批 hashicorp 的图报 **206**，于是判定"复活了、不能删"，还顺手给出了
  "站点当时可能在维护或限流"这种解释。
- 真相是**服务器把 404 错误页的前 1KB 用 206 发了回来**：

  ```
  带 -r 0-1023 :  206  text/html   1024 B     ← 错误页的前 1KB
  普通 GET     :  404  text/html  58156 B     ← 真相
  ```

  UA 不是变量，两种 UA 结果一样。是 **Range 本身**制造了假阳性。
- 更糟的是这个探测方式在整轮清理里到处用过，所以那份"多少活、多少死"的分类
  整体不可信 —— 而它差一点成为**删除**的依据。
- 规则：
  1. **判断资源是否存在，用普通 GET，并且看 content-type**。只看状态码不够：
     404 页面也可以是 200/206，图片路径回 `text/html` 就说明它已经不是图了。
  2. **破坏性操作的依据，要用将要执行它的那个工具、在同一时刻产出的结果**，
     而不是另一个工具在另一个时间的读数。最终删除依据换成了 `rehost-images`
     自己那一轮的失败记录，且只取**明确 404**：网络失败(29)、401/403(3)、
     5xx(2) 一律排除 —— **"取不到"不等于"不存在"**。

### 安全栅栏也有代价，而它的代价常常是「另一条路径坏掉了」

- CI 那把部署密钥曾被 `authorized_keys` 的 `restrict,command=` 钉死，泄漏也只能在
  **已发布版本之间**挪生产。这个设计写在两份文档里，标着「重构时不能扔」。
- 它配套的机制是：节点自存一份 compose，CI 传 tag 上那份的 sha256，不一致就拒绝。
- 结果是**每一次 compose 改动**（配额、开关、备份变量）都让下一次部署失败，直到有人
  上笔记本手工兜底。于是「兜底路径」变成了实际主路径 —— 一条没人设计过、没人测过、
  CI 跑不了的主路径。
- 而栅栏挡住的那扇门旁边还有一扇开着的：同一个 CI 握着镜像仓库推送凭据，被攻破的 CI
  可以推一个恶意镜像，再用那把「只能部署已发布版本」的密钥正大光明地部署它。
- 它还在挡别的事：备份恢复演练的自动化被卡了几个月，理由逐字就是这道栅栏。
- 规则：评估一道安全边界，**两边都要算** —— ①它挡住的攻击，在同一威胁模型下是不是
  已经有别的路可走；②它让哪条正常路径不可用，以及人们绕过它时走的是什么路。
  一道让主路径失效的栅栏，实际效果是把工作赶到没有防护的旁路上。
  拆掉时**把净损失写下来，别粉饰**（这次是：那把 key 现在有 shell 了）。

### 「跑一遍但不改任何东西」是能力不是形式；它当场抓到两个会炸生产的错

- 部署逻辑从 `ssh "cd … && sed -i … && docker compose …"` 换成 Ansible playbook 时，
  真正决定性的不是 YAML 比 shell 整洁，是 `--check --diff` 能**对着真生产节点**跑完
  整条流程而不动任何东西。
- 第一次彩排就报出两个错，两个都会在上线当场炸：
  1. `.env` 写 `MICA_VERSION=0.13.27`，而节点上是 `v0.13.27`。compose 直接拿这个值
     当镜像 tag（`mica-api:${MICA_VERSION}`），少个 `v` 就是去拉一个不存在的 tag。
  2. `docker compose up -d postgres` 会**重建 `mica-postgres-1`** —— 在跑的容器是用
     更早的 compose 创建的，config-hash 对不上。等于每次发版弹一次数据库。旧脚本写
     `--no-deps api web`，我当成了优化，它其实是不变量。
- 第二个错尤其说明问题：它**不可能靠读代码发现**，因为它取决于节点上那些容器当初是
  用哪一版 compose 创建的 —— 那是只有生产节点自己知道的状态。
- 附带一条：彩排要有价值，读操作必须 `check_mode: false`。否则干跑跳过它们、后面的
  断言在空数据上失败，报的是干跑自己的假象 —— **一个因为真实执行不可能出现的原因而
  失败的彩排比没有彩排更坏**，人会学会忽略它。
- 规则：改造部署路径时，**先问「有没有办法把它跑一遍而不生效」**。有的话，那个能力
  本身通常比这次改造要解决的问题更值钱。

### 我把生产凭据打进了公开仓库的 CI 日志 —— 靠一个「只是读一下状态」的循环

- `community.docker.docker_container_info` 返回的是**整个 inspect payload**，里面有
  `Config.Env`，也就是那个容器的**实时凭据**。我用它检查 postgres/rustfs 是否健康。
- Ansible 会把 **loop item 附在每一条 per-item 结果上**。所以下一个 `assert` 循环
  `data_info.results` 时，把 item 连同凭据一起打了出来 —— **不需要 `--diff`，不需要
  `-v`，默认 verbosity 就会打**。
- 后果是真的：三次 Deploy 运行把 `POSTGRES_PASSWORD`、`RUSTFS_ACCESS_KEY`、
  `RUSTFS_SECRET_KEY` 的明文写进了 **公开仓库** 的 Actions 日志。而 rustfs 的 `:9000`
  是有意对外的，那对密钥公开出去等于**桶可写**。
- 处置顺序：① 先改代码让它不可能再发生 → ② 删掉那三次运行 → ③ **轮换三个凭据**。
  ③ 不可省：**「日志删了」不等于「没人看过」**，公开仓库的日志随时可能已被抓取。
- 修法**不是加 `no_log` 了事** —— 那只藏显示，数据还在变量里，下一个不小心打印它的
  任务照样泄。真正的修复是**把数据削掉**：`no_log` 收下 inspect 结果后立刻 reduce 成
  `{name, running, health}` 三个标量，之后任何任务想打也打不出凭据。
- 附带三个同源的坑，都在这一次里踩到：
  1. `--diff` 会打印模板任务的**旧文件全文**。`.env` 里有凭据时，`--check --diff`
     就是一条把密钥打进 CI 日志的命令。
  2. 我为了定位泄露，把命中行**截断 60 字符**打了出来 —— 其中一个值短到完整落进了
     记录里。**截断不是脱敏**。
  3. 「旧密码应该失效了」我第一次是从容器里连 `127.0.0.1` 验的，而 pg_hba 对本地是
     `trust`：**任何**密码都返回成功。那条测试用一个明知错误的密码「通过」了，
     等于没测。改成从另一个容器经网络连（`scram-sha-256`）才是真的。
- 规则：**任何会返回「对象完整状态」的只读调用，都要先假设它带着凭据**。要么当场削成
  你真正需要的几个标量，要么就别把它放进任何会被循环、打印、diff 的地方。
  「我只是读一下状态」是这类泄露的标准开场白。

### 把逻辑从 justfile 搬进脚本时，「由调用方注入的变量」变成了隐式契约

- 迁移把 `just` recipe 体搬进 `scripts/*.sh`，recipe 里的常量（`{{site}}`、`{{node}}`、
  `{{flutter}}`）改成由 justfile 注入的环境变量。只要一直经 `just` 调用，这没问题。
- 直到 `scripts/deploy-prod.sh` 开始**直接**调 `verify-prod.sh`。后者在 `set -u` 下写的是
  `${SITE}`，于是**第一次真部署**失败在最后一行：
  `scripts/verify-prod.sh: line 3: SITE: unbound variable` ——
  而 playbook 自己已经 `ok=20 changed=0 failed=0` 跑完了。
- **这个方向的错更贵**：一次成功的部署被报成失败，会引人重跑、或者去生产里找一个并不
  存在的问题。（反方向——失败报成成功——更危险，但这一个更容易被当成"小毛病"放过。）
- 规则：**提取代码时，原来由调用方提供的每个值都要有默认值**，否则你把一个编译期就
  确定的依赖换成了运行期才发现的依赖。判据很简单：这个脚本能不能**单独**跑起来？
  跑一次就知道。当时三个脚本有这个形状，我只在生产上撞见了其中一个。

### 一条需要写进 CLAUDE.md 提醒自己的规则，不是规则，是指望

- 「发版前跑测试必须带 `DATABASE_URL`，否则 DB 集成测试静默跳过还报『全过』」这条，
  在 CLAUDE.md 里当提醒挂了很久，每次发版都要先想起它。用户的原话是：
  「这个能不能放在标准发版流程里，我看你每次都要提醒下自己」。
- 修法不是把提醒写得更醒目，是**让顺序不再可选**：`just release X.Y.Z` 把
  bump → 门禁 → commit → tag 焊成一步，门禁夹在中间，本地 Postgres 没起来当场拒绝。
- 顺带发现那条提示本身就是错的：老 doc string 写着「先跑 release-check 再 bump」，
  而 release-check 断言三处版本号一致 —— 它们只有 bump **之后**才一致。也就是说
  那条要人记住的规则，连它自己描述的顺序都跑不通，**而这一点从没被发现过**，因为
  没人真按它写的顺序跑过。
- 规则：**发现自己在给某条规则加提醒时，先问它能不能变成一道拒绝。** 提醒的失败率是
  人的记忆力，拒绝的失败率是零；而且写拒绝会逼你把规则说清楚，说清楚的过程本身就能
  暴露这条规则是不是真的成立。


### 门禁写在作者手上的状态,而不是调用方会造出的状态

同一天,同一个脚本(`scripts/release-check.sh`),第二次撞上「这道门永远过不去」。

- 第一次:`flutter analyze` 不带 flag,而这个包有约 130 条 `info` —— 它永远退出 1。
- 第二次:第 4 条断言 `git diff --quiet Cargo.lock`(「lock 未被修改」)。而
  `release.sh` 的**第 1 步**就是 `release-bump.sh`,它最后一行 `cargo check` 存在的
  唯一目的就是刷新 lock;提交是第 3 步。**门禁挡住的正是它自己要求创建的状态。**

两次形状完全一样:**断言写的是作者当时手上的状态**(干净工作树、只跑 `analyze` 那一下),
**不是调用方会造出的状态**。而且两次都"验证过" —— 单独跑 `just release-check` 都过,
只在它本来要服务的那条流程里失败。

`just release` 是 2026-08-25 加的,到 2026-08-26 真发一版才第一次跑通 —— 也就是说
这条流程从加进来到被真正使用之间,一次都没有端到端跑过。

规则:**一道门禁必须从调用它的入口跑一遍**,不能只单独跑。「它自己能过」和「流程能过」
是两个事实,中间隔着的正是这道门存在的理由。

### 「装上了」不等于「能用」:Windows 上的 ansible

`scripts/deploy-prod.sh` 检查 `command -v ansible-playbook`,通过就往下走。在 Windows 上
`pipx install ansible-core` **会成功**,`ansible-playbook.exe` 也确实在 PATH 上 ——
然后一运行就死在 ansible 自己的启动里:

    File ".../ansible/cli/__init__.py", line 46, in check_blocking_io
    OSError: [WinError 87] 参数错误。

ansible 只支持 Linux/macOS 作控制端。于是这个检查的效果是**把一句能读的拒绝,换成一段
深处的 traceback**。改成 `ansible-playbook --version` —— 检查能力,不检查文件在不在。

顺带暴露的是**文档漏了一条平台前提**:`just deploy-prod` 在 2026-08-25 的改造里被反复
讨论、写进三份文档、称作「和 CI 走同一条路径」,而三份都没说它要跑在 Linux 上 ——
于是在一台 Windows 主力机上,它看起来是可用的。**「安装命令」不等于「运行环境」**:
写前置依赖时把它跑在哪种机器上一并写出来,否则读的人默认是自己这台。

### `gzip -t` 证明不了那是个备份

迁移前还原点那段,第一版校验写的是 `gzip -t`。实测(2026-08-26,本地 dev 库):

- `pg_dump -d 不存在的库 | gzip > f` —— 失败的 dump 留下一个**完全合法的空 gzip**,
  `gzip -t` **通过**。
- 把真 dump 截掉一半再压 —— `gzip -t` 也**通过**。

`gzip -t` 校验的是压缩容器,不是内容。判据换成「pg_dump 的结尾标记 **且**
`COPY public.document_yrs_base` 同时在内」,上面两种都被拒。

同一段里另外两个实测出来的坑:

- **`pg_dump | gzip` 不带 `set -o pipefail` 时,dump 失败整条管线返回 `0`**
  (实测 `rc=0` vs 带 pipefail 的 `rc=1`)。所以那个 `-o pipefail` 是承重的,不是风格。
- **`zcat f | grep -q ...` 在 pipefail 下返回 141**:`grep -q` 命中就退出,`zcat` 吃到
  SIGPIPE,而 pipefail 把它读成失败。所以校验用一趟 `awk` 读到底,不用 `grep -q`。

规则:**写「验证」的时候,先构造一个应该被拒的样本喂给它。** 如果构造不出来,
这个验证多半什么都没验。
