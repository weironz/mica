<!-- codebase-memory-mcp knowledge graph -->

## 探索代码:先用知识图谱

本项目有 codebase-memory-mcp 知识图谱(project `C-data-codes-mica-will-laptop`)。
**探索代码先用它,再退回 Grep/Glob/Read** —— 更快、更省 token,而且给得出文件扫描给不了的
结构信息(调用者、被调、数据流、测试)。

| 想做什么 | 用 |
| --- | --- |
| 按名字/标签找函数、类、路由 | `search_graph` |
| 全文搜索(替代 grep) | `search_code` |
| 读某个符号的源码 | `get_code_snippet` |
| 调用链 / 影响面 | `trace_path`(calls\|data_flow\|cross_service) |
| 复杂关系(调用者、导入、测试) | `query_graph`(Cypher) |
| 高层结构 | `get_architecture` |
| 改动风险评估 | `detect_changes` |

**图谱不会自动更新**:改动较大、或 `search_graph` 找不到刚加的符号时,手动跑
`index_repository(repo_path="C:/data/codes/mica-will-laptop", mode="full")`(`full` 慢但含语义边;
另有 `moderate`/`fast`),用 `index_status` 看新鲜度。查询要带
`project="C-data-codes-mica-will-laptop"`。`full` 偶发 worker 崩溃(崩溃日志是空文件),原样重试一次即可。

## 项目原则(长期有效,优先级高于默认习惯)

1. **In-house 优先** —— 最小化第三方依赖,宁可自研(编辑器整个是自绘的)。**引入依赖需明确豁免**,
   现有豁免见下表。in-house 该留给**核心数据面**(CRDT / 文档模型 / 同步),不是平台粘合层:
   粘合层自研要背三套平台原生层,用成熟包反而对。
2. **Rust-first 数据面** —— 数据处理一律在 Rust;Dart 只做 UI 和编辑器热路径。Markdown 语法逻辑
   两端必须同步,**Rust `crates/markdown` 是权威**,Dart `editor/marks.dart`/`markdown.dart` 是镜像。
3. **渲染架构红线** —— 新渲染能力先抽象机制(`AtomicBlockRenderer` 注册表),**严禁往 `render.dart`
   堆 if 分支**。见 `docs/render-architecture.md`。
4. **Markdown 方言** —— CommonMark 0.31.2 底座(读侧 641/641)+ GFM(24/24)+ 方言(脚注、front
   matter、Pandoc 数学)。写侧输出规范化子集,**round-trip 是不变量**。记分牌
   `docs/commonmark-scoreboard.md`,回归地板 `commonmark_scoreboard.rs`。
5. **修复纪律** —— 每个 bug 修复配**回归测试 + 实测验证**(web 用 playwright 截图);提交信息写
   **根因**,不写流水账。
6. **难决策先扒同类产品** —— 没有明显正确解、或自己的方案有明显代价时,拍板前先看别人怎么做。
   重点不是"它支不支持",而是"**相同约束**下它具体怎么做、又刻意**没用什么**"。最该警惕的是自己
   脑子里「必须 X 才能 Y」那类前提 —— 给出"几选一"前先问这些选项的**共同前提**验证过没有。
   参照系:AppFlowy(Flutter 同构)、AFFiNE(web/Yjs)。手段:调研子代理 + GitHub MCP 读真实源码。
   〔两次教训见 `docs/lessons.md#7`:mermaid 曾误以为"服务端渲染必须 headless 浏览器";
   merman 的主题曾误以为"只能改 CSS"。第三次见 `docs/markdown-boundary-comparison.md`:
   曾误以为"web 没 FFI 所以 markdown 必须两端各写一份",实际根因是**服务端要能解析**这条
   产品选择 —— 两家同类都把 markdown 只放客户端,且都用现成解析器。〕

### 依赖豁免清单

| 包 | 用途 | 约束 |
| --- | --- | --- |
| flutter_math_fork | 数学渲染 | 复杂领域渲染器,自研不现实 |
| window_manager | 桌面窗口大小/位置/最小尺寸/拦截关闭 | 迁移评估见 `docs/window-manager-migration.md`(结论:不迁) |
| tray_manager | 系统托盘("关闭最小化到托盘") | **仅 Windows**(`trayIsSupported`);**注册失败必须降级 `minimize()`,绝不 `hide`** —— 否则窗口再也找不回来 |
| file_picker | 桌面文件对话框 | 非 web |
| pasteboard | 桌面富剪贴板图片读写 | 非 web |
| merman | 纯 Rust headless mermaid 引擎(FFI) | 离线渲染;配色走它的 host theme profile |
| flutter_svg | 把 merman 的 SVG 栅格成 `ui.Image` | 非 web |
| yjs(JS) | **web-only** CRDT 引擎 | Rust `yrs` 的 JS 对端,编码层字节兼容(已实证);只进 web bundle |
| xml / html | 纯 Dart 解析库 | — |

除 flutter_math_fork / yjs 外,**均只在 `_stub`/`_web` 变体里用**,条件导入隔离,不进 web bundle。
merman 的 SVG 主题用 CSS 而纯 Dart 渲染器不解析 → 自研 `mermaid_svg_inline.dart` 把 CSS 拍平进属性
(merman 文档把这列为 host 边界)。

## 当前状态(2026-07-30,v0.13.5)

- **注册默认关闭**(`MICA_REGISTRATION_ENABLED`,只有显式 `true/1/yes/on` 才开;拼错保持关闭)。
  开启时注册返回 **204 而非 session** —— 需邮箱验证后才能登录。空实例的**第一个账号**永远放行
  且直接标记为已验证(否则全新自托管装不起来)。每工作区字节配额默认 **5 GiB**
  (`MICA_WORKSPACE_QUOTA_BYTES`)。**注意**:节点 `.env` 里设了还不够 ——
  `deploy/docker-compose.yml` 的 `environment:` 是显式允许清单,没列进去的变量到不了进程。
- **部署零凭据可起**(2026-08-06,v0.13.16):`JWT_SECRET` 不设时服务端首启自铸 32 字节存进
  `server_secrets`(migration 0020)、之后复用,设了则仍走严格校验;`POSTGRES_PASSWORD` 默认
  `mica`(两份 compose 都不发布 postgres 端口,安全);**S3 那对也给了默认**
  `mica`/`mica-default-not-a-secret` —— ⚠️ **这一个不安全,是用户 2026-08-06 明确拍板的取舍**:
  rustfs `:9000` 有意对外(浏览器直接 presign),默认值又写在公开仓库里,等于装机不改就是
  可写的桶。所以生产环境用着默认值时 `AppState::new` 会 `warn!`(`default_s3_secret_in_use`)——
  只写在文档里的风险等于没写。另:**建桶由服务端走 S3 接口做**(`bucket::ensure_bucket`,
  启动时 HeadBucket→缺失才 CreateBucket,**任何非 404 都当"存在"、且永不阻断启动**)——
  不绑 RustFS,换 MinIO/OSS/S3 都成立;方案见 `docs/bucket-provisioning-plan.md`。
  ⚠️ **"没设"在容器里有两种形态,别只测其中一种**:compose 的 `${VAR:-}` 把未设变量解析成
  **空字符串**,`env::var` 因此返回 `Ok("")` 而不是 `Err` —— 自铸功能第一版就栽在这:手跑二进制
  (变量真的不存在)能自铸,一进 compose 全部 crash-loop。**空白必须等同于未提供**
  (`resolve_jwt_secret`)。另外 `${VAR:?}` 会**先于**进程拒绝空值,改 compose 时一并注意。
  本地(离线)模式**不涉及**:它走 FFI 直连本地库,没有 session/token 这一层。
- **op 模型已完全退役**(S0–S5,migration 0016):`document_snapshots` / `document_updates` /
  `document_versions` 三表已删,文档内容只存在于 `document_yrs_base`。
- **web 端有 e2e 了**(`just web-e2e` + CI `web-e2e` job):浏览器里的 yjs 与服务端 yrs 经真 WS
  收敛、服务端渲染路由压过 SPA 兜底、入口文件缓存头、POSIX locale 下仍能启动。
- **Web 稳定**,Markdown 规范线已闭环;桌面端 **Windows 优先**。
- **深色主题已完成**:语义色 token 层贯穿外壳 / 自绘画布 / 语法高亮 / mermaid,跟随系统或手动切换。
- 登录页与首页按设计稿重做,桌面与 web 同一屏。
- **平台现状**:Windows 出安装包并支持自动更新;`linux/` 有目录但 CI 不出桌面包;无 macOS / 移动端。
  本地(离线)模式是桌面独有,web 没有。
- **待办总账在 `docs/roadmap.md`**(权威,**只放还要做的** —— 拍板不做的**整条删掉,不留**
  〔用户 2026-08-05 定〕,理由进那次提交的信息,`git log` 查得到;代价是同一个决定可能被
  重新翻出来做一遍,拿不准就先 `git log --grep` 搜一下);
  已完成的整条存档在 `docs/roadmap-done.md`(只进不出,发版时搬,见 release.md 步骤 5);桌面计划与开发环境备忘
  (stale bundle、幽灵会话、DB 取证)在 `docs/desktop-plan.md`。

## 架构速记

- **编辑器**:单 RenderBox 自绘画布(`render.dart`),marks-over-plain-text 模型,IME 走
  `TextInputClient`;硬换行的存储约定 = 文本里的 `\` + 换行。
- **页树不变量:folder 是唯一容器,page 是叶子**。服务端任何写 `parent_view_id` 的路径**必须**过
  `documents::ensure_parent_accepts_children`(400 + 可读原因),DB 触发器
  `views_parent_must_be_folder` 兜底(migration 0011)。〔这条以前只活在 Dart 客户端,
  Notion 导入就造出 137 个"页面下挂页面" —— `docs/lessons.md#2`。〕
- **块级嵌套是扁平模型**:`data.indent`(列表层级)、`data.quote`/`qbreak`(引用深度/分组)、
  `data.li`(item 容器子块);HTML 导出端重建嵌套。
- **代码字体**:web 上 `'monospace'` 族名不解析,一律用 `kMonoFont`(打包的 Roboto Mono,`model.dart`)。
- **快捷键**:清单在 `docs/shortcuts.md`(权威)。加/改要**四处同步**:`editor.dart` key handler +
  `main.dart` `_appShortcuts` + `dialogs.dart` `_shortcutsSection` + 该文档。
- **踩过的坑见 `docs/lessons.md`,和本文件一起读**:双表示、不变量只写在客户端等于没写、测试真空
  通过、web 通过≠桌面通过、图片 dispose 时序、round-trip 红线。本文件写**规则**,那份写规则的
  **来历** —— 规则很容易被当成"大概是这个意思"绕过去。

## 发版(权威文档 `docs/release.md`)

`just release X.Y.Z`(bump → 门禁 → commit → tag)→ 推 tag → CI 产出全部 7 个产物 →
手动触发 `Deploy` workflow 上线。**手动那几下的完整清单在 `docs/release.md` 顶部**。
彩排:`gh workflow run Deploy -f version=X.Y.Z -f check=true`,对真节点跑一遍不改任何东西。
`just --list` 看全部 recipe。

⚠️ **`just deploy-prod`(兜底)在本机跑不了**(2026-08-26 实测):ansible 不支持 Windows 作
控制端,`pipx install ansible-core` 装得上、一跑就 `WinError 87`。所以「GitHub 挂了怎么办」
**今天没有答案** —— 要这条路得先装真的 WSL 发行版(系统级改动,由用户决定)。

**节奏(用户定,长期有效)**:

- 改动做完后**推送 `main` 由你自动完成**,不用问。
- **是否发版由用户决策**(等用户说「发版」)。用户一说发版,**后面一条龙由你做完**:版本号 bump →
  打 tag → CI → 部署 → 验证,不用再等第二次指令。
- 版本号**三处必须一致**:`pubspec.yaml` / `main.dart` 的 `kAppVersion` / 根 `Cargo.toml`
  (api-server 与 mica-cli 都继承它;顺带 `cargo check` 更新 `Cargo.lock`)。
- 补丁位递增(新功能也走补丁),**minor 由用户拍板**。
- ~~发版前 `just test` 必须带 `DATABASE_URL`~~ —— **2026-08-25 起变成门禁,不再靠记性**:
  `just release` 必经 `scripts/release-check.sh`,本地 Postgres 没起来就当场拒绝。
  (原因仍然成立:DB 集成测试在 `DATABASE_URL` 没设时**静默跳过**,而跳过的测试报告为
  「通过」。一条需要写在这里提醒自己的规则,本身就说明它不是规则。)
- 上线后验证 `/api/health` 报对版本,**并冒烟测这一版真正改了什么** —— 版本号证明不了功能。
- **发版提交里同步 `docs/roadmap.md`**:这一版关掉的条目当场标掉 —— 「无 X」这类否定式条目
  只会静默变假,发版是唯一的执行点(release.md 步骤 4)。

## 开发机与工具链(Windows 主力机)

- 仓库在 `C:\data\codes\mica-will-laptop`(2026-07-20 起)。旧文档里的 `D:/codes/mica` 一律按本路径理解。
- Flutter stable 在 `C:\flutter`(已进 PATH);VS **Build Tools** + Windows SDK 已装,
  `flutter build windows` 可用(不需要 VS Community)。Flutter app 在 `clients/mica_flutter/`,
  仓库根是 Rust workspace。
- **国内网络必须走镜像**:已设 `FLUTTER_STORAGE_BASE_URL=https://storage.flutter-io.cn` +
  `PUB_HOSTED_URL=https://pub.flutter-io.cn`,否则 `flutter pub get` 会卡住。
- **`just` 的 shebang recipe 绕过 `set windows-shell`**(里面用 `cygpath`):跑之前
  `export PATH="/c/Program Files/Git/usr/bin:$PATH"`,或直接在 Git Bash 里跑。
- 强制重启/断电后 `.dart_tool/flutter_build` 缓存可能被截断 → frontend_server 报 `RangeError`。
  `rm -rf clients/mica_flutter/.dart_tool/flutter_build` 即愈。
- **构建按标准来,不做热限流**:全量 `flutter build windows` 该跑就跑,`cargo` 用默认并行度。
  (上一台机器的硬卡死是**设备自身故障**,不作为本机约束。)能交 CI 仍是好习惯,但那是工程取舍。
- **Docker Desktop 非必要不启** —— 理由与热无关:WSL2 冷启动 100s+,且 `docker info` 在启动中会
  **阻塞**而非快速失败,别用十秒超时判"卡死"。
- **runner C++(`windows/runner/*.cpp`)是 warning-as-error**:注释必须纯 ASCII(CJK 撞 C4819),
  `_wgetenv` 已弃用要换 `GetEnvironmentVariableW`。

## 生产运维

节点 `mica.cloudcele.com`(阿里云),key 认证免密。容器名 **`mica-postgres-1`**(不是 `mica-postgres`,
那是本地 dev 栈的)。详见 `docs/deploy.md` / `docs/release.md`。

- ~~⚠️ **`deploy-prod` 不做迁移前备份**~~ —— **2026-08-26 起 playbook 自己做**:比一次
  `git diff v<节点当前版本>..v<新版本> -- migrations/`,**有新迁移才**落
  `/data/mica/pre-<版本>-<ts>.sql.gz`,问不出答案时按「有」处理。校验不能只用 `gzip -t`
  (**实测**:失败的 dump 会留下一个合法的空 gzip,截断的也照过),判据是 pg_dump 结尾标记 +
  `COPY public.document_yrs_base` 同时在内;`pg_dump | gzip` 那条管线**必须** `set -o pipefail`
  (不带的话 dump 失败仍返回 0,实测)。任何一步不过就删半成品并**中止部署**。
  backup sidecar 仍然当不了回滚点(周期导出器,api 起来之后才刷)。
- **分层生效**:服务端改动随 api 部署即生效;**MCP 代理层的改动在 `mica-cli` 二进制里**,用户不把
  MCP 指向新版并重连就还是旧行为。排查"我明明改了怎么没生效"先分清这层。
- **迁移是 `sqlx::migrate!` 编译期嵌入的**:新增迁移文件不触发 `mica-infra` 重编 →
  加迁移后 `touch crates/infra/src/db.rs`,否则 `run_migrations` 还带旧集合。

## 协作约定(用户定,长期有效)

- **对话与项目文档用中文**(README 面向公众用英文)。
- **不要过度设计**:按需求的最小充分解做 —— 不为假想的未来需求预先抽象/加旋钮/加层次。够用、
  可验证、边界诚实优先于"完备";看到更简单的做法先质疑复杂方案,拿不准就选小的,并把
  **"为什么先不做"写清楚**。
- **计划批准后连续执行到完成**,中途决策按推荐项走,不逐步请示;**诚实报告做不到的部分**。
- **能用子代理/workflow 提效就主动用**。但**"边改文件边攒清单"这类批量活必须单 agent 顺序跑完、
  不打断** —— 子代理的文件编辑是真实落盘,被打断时清单随之丢失,留下一堆悬空引用
  (i18n 那次并行派三个、拒了两个,136 个 key 的翻译全丢)。
