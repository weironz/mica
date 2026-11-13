# Mica 路线图 — 未完成事项

> 这份文件只放没做完的事；完成项整条搬入 [roadmap-done.md](roadmap-done.md)。
> 维护规矩：
> 1. 完成即搬走，不能只打勾；发版和询问下一步时逐条复核。
> 2. 「无 X」「不校验 Y」等否定式条目须对照代码核实，不能靠记忆。
> 3. 核实结论写在条目内（日期 + 依据）；不另维护容易失真的重复计数表。
> `[等用户]` 表示仅用户能解锁的发布、策略或实机验证；不是尚未排期。
> 2026-11-13（0.14.6 发版前）复核：以下事项仍未完成；本轮界面改造记录见已完成存档。

## 平台

- 🔭 GPUI UI 转型候选 [#172](https://github.com/weironz/mica/issues/172)，未立项，当前不迁。
  2026-11-10 部分验证的 Web 中文输入、桌面中文输入、性能和移动四关均未完整通过；
  Windows 真实输入法、Android 真机、Safari/Firefox 仍待验证。
  见 [调研与决策](../architecture/ui-gpui-evaluation.md)。

## SDK 与代码结构

- `[等用户]` `mica-md` crate/npm 公开发布（S，2026-11-01 核实）。
  SDK 已就绪且保留 `publish = false`，严格 HTML 档已完成；由用户批准并亲自发布。
  见 [S5 发布手册](../single-source/stages/S5-sdk-release.md)。
- 渲染库真正拆库（2026-11-01 评估后暂缓）。块渲染器依赖 `RenderDocument` 26 个私有字段，
  下次新增块渲染器时先设计上下文接口；目前仅超软上限。见 [渲染架构](../architecture/render-architecture.md)。

## 交付验证

- 发布产物 canary（2026-11-01）：从 Release 下载校验、Windows 隔离安装启动、线上 `/api/health`
  和固定入口已接入发布流程；仍需补上从上一版安装覆盖升级的自动验证。

## yrs 输入兜底

- `[需后端]` 等上游修复剩余普通 panic 后，重新验证并启用 `proptest_yrs.rs` 四条 ignored property。
  2026-10-08 之后已接入官方 yrs 0.28.0；异常分配和非法 UTF-8 已有默认回归，
  但 `block.rs:92` 仍可 panic。跟踪 [#415](https://github.com/y-crdt/y-crdt/issues/415)。
  服务端 `guarded_from_update` / `guarded_apply`、FFI `contain_yrs_panic`、CRC 与损坏缓存自愈继续保留。
  不据依赖升级改变注册策略，也不宣称任意输入全面安全。
