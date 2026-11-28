# Mica 路线图 — 未完成事项

> 这份文件只放没做完的事；完成项整条搬入 [roadmap-done.md](roadmap-done.md)。
> 维护规矩：
> 1. 完成即搬走，不能只打勾；发版和询问下一步时逐条复核。
> 2. 「无 X」「不校验 Y」等否定式条目须对照代码核实，不能靠记忆。
> 3. 核实结论写在条目内（日期 + 依据）；不另维护容易失真的重复计数表。
> `[等用户]` 表示仅用户能解锁的发布、策略或实机验证；不是尚未排期。
> 2026-11-28 复核：安装包的隔离与上一版覆盖升级冒烟（`scripts/smoke-installer.ps1 -UpgradeFrom` / `scripts/release-canary.sh`）已于 2026-11-25 闭环并移入 [roadmap-done.md](roadmap-done.md)；当前路线图仅余以下 2 项外部依赖项。

## SDK 与代码结构

- `[等用户]` `mica-md` npm 公开发布（S，2026-11-28 核实：crates.io 已发布 [`mica-board 0.1.0`](https://crates.io/crates/mica-board) 与 [`mica-md 0.1.0`](https://crates.io/crates/mica-md)，tag `mica-md-v0.1.0`；仅余 `packages/mica-md-js` 待用户执行 `pnpm adduser --registry https://registry.npmjs.org` 与 `pnpm publish --access public`）。
  见 [S5 发布手册](../single-source/stages/S5-sdk-release.md)。

## yrs 输入兜底

- `[需后端]` 等上游修复剩余普通 panic 后，重新验证并启用 `proptest_yrs.rs` 四条 ignored property。
  2026-10-08 之后已接入官方 yrs 0.28.0；异常分配和非法 UTF-8 已有默认回归，
  但 `block.rs:92` 仍可 panic。跟踪 [#415](https://github.com/y-crdt/y-crdt/issues/415)。
  服务端 `guarded_from_update` / `guarded_apply`、FFI `contain_yrs_panic`、CRC 与损坏缓存自愈继续保留。
  不据依赖升级改变注册策略，也不宣称任意输入全面安全。
