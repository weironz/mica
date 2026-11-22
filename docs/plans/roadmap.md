# Mica 路线图 — 未完成事项

> 这份文件只放没做完的事；完成项整条搬入 [roadmap-done.md](roadmap-done.md)。
> 维护规矩：
> 1. 完成即搬走，不能只打勾；发版和询问下一步时逐条复核。
> 2. 「无 X」「不校验 Y」等否定式条目须对照代码核实，不能靠记忆。
> 3. 核实结论写在条目内（日期 + 依据）；不另维护容易失真的重复计数表。
> `[等用户]` 表示仅用户能解锁的发布、策略或实机验证；不是尚未排期。
> 2026-11-16（0.14.9 发版前）复核：以下事项仍未完成。安装包冒烟隔离已实现，但上一版覆盖升级验证仍未补齐；侧栏调整、Web 冷启动和浏览器发布门禁不对应其他既有条目。

## 平台

- 🚀 **全栈 Rust 三层标杆架构重构** [#172](https://github.com/weironz/mica/issues/172)（2026-11-22 核实，分支 `feat/fullstack-rust-architecture`）。
  确立「L1 数据引擎 + L2 无头纯 Rust 客户端内核 + L3 路线二-B（自有 `mica-editor` + NSH 原生壳）」四阶段演进路线，分支已完成核心架构落地，待外部闸门与合入 `main`：
  - Phase 0（本机探针完成；待 `[等用户]` 真人 IME/真机抽检）：纯 Rust Web WASM 1.39 MiB Brotli-11、2,000 块路线二-B 挂载 15.27 ms / 单块更新 17 µs、500 节点 Canvas 1.12 ms；
  - Phase 1（L2 内核 + FFI/WASM 桥接已完成）：`crates/mica-board::session`（100 测试，`board-wasm` 195.1 KiB gzip-9）与 `crates/mica-sync`（66 测试，含 SQLite + 内存快照镜像）；
  - Phase 2（L2 内核 + FFI 句柄已完成；Flutter 旧控制器瘦身与棘轮收紧待续）：`crates/mica-editor`（26 套件含 proptest）与 `crates/mica-client`（7 测试），`clients/mica_flutter` 新增 FFI 句柄与跨平台 `BoardSessionBridge`（`l2_rust_ffi_test.dart` 及核心回归 27/27 通过）；
  - Phase 3（`clients/mica_rust` 原生壳原型与 `app_flows.rs` E2E 全绿，Windows 二进制可编译运行；Flutter 退役等待上游 `nsh-compat` 冻结与物理 Android/Safari/真人 IME 硬闸门签收）。
  详见 [设计方案与分阶段开发计划](../architecture/fullstack-rust-architecture.md)、[P0 实证报告](../reviews/fullstack-rust-p0-spike-2026-11-22.md) 与 [前序调研](../architecture/ui-gpui-evaluation.md)。

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
