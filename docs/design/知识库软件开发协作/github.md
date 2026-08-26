repo: weironz/mica
branch: main
path: clients/mica_flutter/lib

> ⚠️ **不要把这个路径同步成本目录下的文件副本。** 2026-07-27 那次同步在
> `clients/mica_flutter/lib/` 下留了 5 个源文件的快照,之后没人刷新过;到 2026-08-26
> `main.dart` 已落后 5616 行(48%),而设计稿漏掉大量已有功能,几乎可以肯定就是因为
> 设计时读的是它。快照已删除,完整理由在 `docs/design-adoption.md` §2。
>
> 上面三行留着是**指路**——代码在哪。需要代码上下文就去仓库现读,别在这里存一份:
> 一份要靠人记得手动刷新、而实际没人刷的快照比没有更糟,它看起来像代码事实。

## Last sync
date: 2026-07-27T02:14:48Z (代码快照已于 2026-08-26 删除,不再保留本地副本)

### Updated in this project
- 新增「Mica 新功能同步」：文档视图汇总已上线但设计里缺失的 UI
- 评论面板（380px，未解决/孤儿/已解决三态）按 ui/comment_panel.dart 复刻
- 同步徽标三态（已同步不画 / 同步中慢转 / 离线）按 cloud/sync_status.dart
- 页面属性面板、反向链接面板、查找替换栏、字数角标按各自源文件对齐
- 配色统一到 EditorTheme（text #24292F / muted #57606A / faint #9AA4AF / accent #2563EB / codeBg #F4F4F6 / commentHighlight amber 20%）
- 设置·账户与安全：删除账号（密码门控确认弹窗）、修改密码、忘记密码邮件流、源代码（AGPL-3.0）入口，文案取自 l10n/app_zh.arb
- 暗色模式为提案（仓库尚未实现，roadmap 明确全 app 浅色）：给出 EditorTheme 明→暗 token 对照

## Screen map
| 设计文件 | 源文件 |
| --- | --- |
| 02 工作区总览.dc.html | main.dart 工作区列表（卡片/目录树两形态）|
| 07 新功能同步.dc.html | ui/comment_panel.dart, cloud/sync_status.dart, editor/property_panel.dart, main.dart (_BacklinksPanel, _SyncBadge), editor/editor.dart (_buildFindBar, _buildWordCount), ui/dialogs.dart + l10n/app_zh.arb (账户/删除账号/忘记密码/关于), docs/roadmap.md (暗色模式未实现) |
| 03 首页+侧栏.dc.html | main.dart 侧栏/工作区切换，首页「创建页面」入口 |
| 01 登录引导.dc.html | main.dart 登录 + 本地/云端模式 |
| 13 设置.dc.html | ui/dialogs.dart + l10n/app_zh.arb（账户、修改密码、删除账号）|
| 08 版本历史.dc.html | docs/version-history-plan.md |
| 10 实时协作.dc.html | cloud/cloud_sync_session.dart |
| 09 分享权限.dc.html | crates/api-server documents.rs 分享 |
| 14 自动更新.dc.html | updater_desktop.dart |
