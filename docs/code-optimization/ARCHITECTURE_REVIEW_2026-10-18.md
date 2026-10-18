# 架构调整评审与修复（2026-10-18）

## 范围与结论

评审基线 `41809fc7..668192f0`：Rust API 路由、MCP 工具、Markdown 管线拆分，
Flutter shell/editor 拆分与领域控制器抽取，以及新增文件规模门禁。
本次只改 Mica 仓库，不操作生产、不发版。

方向合理：Rust 按已有模块重组，Flutter 用 part 保持库边界，再将状态与异步流程移入
可单测的控制器；Markdown 的完整快照金样本也补上了规范投影测试遗漏的数据结构。
评审重点是异步副作用的生命周期、解析器实际分配，以及门禁是否真的拦截。
文件变小不等于行为保持不变。

服务端 documents 原函数、批量 handler 宏展开、鉴权 helper、导入与 WebSocket 路径，
以及 MCP 工具参数、元数据与五组 router 的核对，未发现可确认的新增行为回归。
这不等于穷尽证明；运行验证见下文。

## 已修复的问题

| 级别 | 根因与影响 | 修复与回归约束 |
| --- | --- | --- |
| P2 | AuthController 抽取后，登录/工作区列表请求在 shell 卸载后仍可写会话及持久凭证。旧流程的 State 更新曾中断这条路径。 | 请求工作区前、返回后检查存活；shell 的两个登录入口返回后也检查。Completer 覆盖两个登录入口、两个等待点。 |
| P2 | ServerRegistry 忽略异步删除回调的 false；等待 outbox 后 shell 已卸载，仍继续删除服务器、凭证及本地镜像。 | 回调明确为 onBeforeRemove，false 中止，非活动服务器返回 true；drain 与切本地之后检查存活。覆盖中止与正常非活动删除。 |
| P2 | CloudViewsController 独立更新 store 后，UI hook 的 mounted 判断无法阻止卸载后的树、ETag 和镜像写入。 | 入口、树响应、正文响应之后检查存活；立即接住 speculative bootstrap 的未消费异常。两个卸载测试在旧实现失败、修复后通过。 |
| P2 | Markdown handler 拆分留下 quote_group 末尾的弃用分类结果；scanner 对 ClassifiedLine 又无条件 clone 两次，额外复制整行文本与 JSON。 | 删除弃用分类及重复缩进归一化；continuation 借用，list handler 通过 ControlFlow 归还未消费的所有权；ClassifiedLine 不再实现 Clone。新增大文本/JSON 指针保留测试，并验证完整金样本。 |
| P2 | main push 的门禁比较 origin/main 与 HEAD，两者通常相同，增量检查为空。 | push 使用事件 before SHA，PR 使用 base 的 merge-base。隔离 Git fixture 证明多提交 push 中的增长不会被漏掉。 |
| P3 | 增量门禁没有源文件路径过滤，大型文档、JSON 金样本也可能因增长被错误拒绝。 | 与绝对限对齐，仅约束 Rust src 与 Flutter lib；覆盖嵌套源文件、文档、测试、JSON、生成豁免、新文件与边界。 |

另修复一项**基线已有问题**：头像上传期间若 token 刷新，旧 session 的 copyWith 会覆盖新
token；若登出或换账号，也可能重新写回旧账号。现在只更新当前同 origin/user 的会话，
保留最新 token 和资料；图片选择器回包也检查账号、origin 和生命周期。
此项没有归因于本次架构调整。

原有 superseded tree 测试没有真正制造旧响应晚到：立即返回再 delay(0)，可能只是新结果
覆盖已经提交的旧结果。已改成 Completer，先提交新树与正文，再完成旧树，检查旧结果
没有写树、ETag、选择和镜像。

## 分配实测

同一 Windows 主机、Rust 1.98.0、release 构建，以临时计数分配器记录一次 import_markdown
期间的累计分配请求字节（alloc/alloc_zeroed/realloc），输入是 2 MiB 单行 ASCII：

| 输入 | 修复前累计分配 | 修复后累计分配 | 减少 |
| --- | ---: | ---: | ---: |
| 普通段落 | 88,081,539 B | 67,109,991 B | 20,971,548 B，约 23.8% |
| 加 `- ` 前缀的列表项 | 109,053,223 B | 88,081,647 B | 20,971,576 B，约 19.2% |

这是累计分配量，**不是进程峰值 RSS**，也不代表生产页面的平均收益。测量程序在验证后删除，
没有把计数分配器或额外依赖加入产品。保留的单测检查分类文本与 JSON 字符串经过拒绝的
handler 后仍是原分配地址；金样本检查解析语义。

## 验证与边界

- Markdown 默认功能完整测试通过：含 CommonMark/GFM、741 输入 × 完整文档/片段两入口的
  全量结构金样本、随机输入属性测试、大文本所有权回归。未重新生成金样本。
- MCP lib 的 30 项测试通过，包括工具列表、参数 schema、只读拒绝与结果语义。
- Auth/Registry 54 项测试通过；Session/Views/Members/Sidebar/Import/Rename/Drop 的
  105 项测试通过。CloudViews 更新竞态测试后，单文件 35 项再次通过。
- 门禁 14 项隔离 Git fixture 与三个脚本 bash -n 通过。
- 完整 Flutter 测试：1,827 项通过、2 项跳过；Markdown 全目标 Clippy deny warnings 通过。
- Flutter 静态分析使用与 CI 相同的 --no-fatal-infos，通过；没有 error/warning，
  仓库现有 info 级提示仍保留。提交后的完整 CI（含 Postgres、Web、桌面集成）另查检查记录。

事件 before/base SHA 缺失时，门禁显式告警并退回 HEAD^，只能覆盖最后一个提交；无父历史
仍执行绝对限及棘轮。不能将回退当成完整多提交检查。

此次没有扩展到全局会话代际重构：restoreSession、跨文档 outbox 扫描、部分跨工作区写操作
仍存在基线已有的时序边界，需要单独构造可复现的跨 origin/workspace 场景再调整；不能把
这些潜在风险当成本次拆分引入的已证实数据损坏。生产搜索、备份和主机资源不在此次评审范围。
