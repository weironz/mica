# Mica 路线图 — 未完成事项

> 这份文件只放没做完的事；完成项整条搬入 [roadmap-done.md](roadmap-done.md)。
> 维护规矩：
> 1. 完成即搬走，不能只打勾；发版和询问下一步时逐条复核。
> 2. 「无 X」「不校验 Y」等否定式条目须对照代码核实，不能靠记忆。
> 3. 核实结论写在条目内（日期 + 依据）；不另维护容易失真的重复计数表。
> `[等用户]` 表示仅用户能解锁的发布、策略或实机验证；不是尚未排期。
> 2026-11-16（0.14.9 发版前）复核：以下事项仍未完成。安装包冒烟隔离已实现，但上一版覆盖升级验证仍未补齐；侧栏调整、Web 冷启动和浏览器发布门禁不对应其他既有条目。

## SDK 与代码结构

- `[等用户]` `mica-md` crate/npm 公开发布（S，2026-11-01 核实）。
  SDK 已就绪且保留 `publish = false`，严格 HTML 档已完成；由用户批准并亲自发布。
  见 [S5 发布手册](../single-source/stages/S5-sdk-release.md)。

## 交付验证

- 发布产物 canary（2026-11-25 更新）：从 Release 下载校验、Windows 隔离安装启动、线上 `/api/health`、固定入口，以及从上一版 Windows 安装包覆盖升级（`scripts/smoke-installer.ps1 -UpgradeFrom` + `scripts/release-canary.sh` 自动查找上一版校验并验证用户 profile/store.db 保留）已全部接入发布流程与契约测试；待下次正式发版在 GitHub Windows runner 首次实跑确认后搬入归档。

## yrs 输入兜底

- `[需后端]` 等上游修复剩余普通 panic 后，重新验证并启用 `proptest_yrs.rs` 四条 ignored property。
  2026-10-08 之后已接入官方 yrs 0.28.0；异常分配和非法 UTF-8 已有默认回归，
  但 `block.rs:92` 仍可 panic。跟踪 [#415](https://github.com/y-crdt/y-crdt/issues/415)。
  服务端 `guarded_from_update` / `guarded_apply`、FFI `contain_yrs_panic`、CRC 与损坏缓存自愈继续保留。
  不据依赖升级改变注册策略，也不宣称任意输入全面安全。
