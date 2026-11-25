# Mica Performance Benchmark Suite (`benchmarks/`)

Mica 全栈纯 Rust 四件套性能基准与压测体系（零额外外部压测框架依赖），用于跨版本回归门禁、大文档/万篇库/多端协同的定量基线对比。

## 目录结构

```text
benchmarks/
├── Cargo.toml                 # workspace 成员 `mica-bench`（publish = false）
├── src/
│   ├── main.rs                # 套件 ②：L1+L2 内核 / CRDT 轨迹 / 万篇 SQLite 知识库 / 500 节点白板
│   └── bin/
│       └── ws_load.rs         # 套件 ④：真实协议 /ws/ 多人协同服务端压测器 + /metrics 采集
├── desktop_latency.ps1        # 套件 ①：Windows 原生桌面冷启动 + Typometer 式黑盒 BitBlt 像素延迟探针
├── web_perf.mjs               # 套件 ③：Playwright + Chromium CDP WebAssembly 自动化性能门禁
├── serve_web_proxy.mjs        # 本地同源静态 + /api /ws 反向代理（供 web_perf.mjs 复用）
├── baselines/                 # 历史 Release 基线报告与 NDJSON 原始快照
└── README.md
```

另配 L3 原生壳无头渲染帧基准：`clients/mica_rust/examples/l3_frame_bench.rs`（可在锁屏或无头 CI 环境下测量 NSH 完整信号+布局+帧提交延迟）。

## 四件套快捷命令

```bash
# ① 桌面原生端冷启动与按键到屏幕像素延迟（需解锁后的 Windows 交互式桌面）
just bench-desktop
# 无头/CI 环境下测 L3 原生壳渲染帧延迟
cargo run --manifest-path clients/mica_rust/Cargo.toml --release --example l3_frame_bench --locked

# ② 纯 Rust L1+L2 内核 + 知识库 + CRDT 轨迹 + 白板
just bench                                          # 标准基线（2k 块 / 20k 步 / 1k 篇 / 500 节点）
just bench-10k                                      # 万篇重压（10k 篇 SQLite 库 + 50k 步 CRDT 轨迹）
just bench --trace path/to/automerge-paper.json     # 回放外部 josephg/editing-traces JSON
just bench --json > benchmarks/baselines/latest.ndjson

# ③ WebAssembly 真实浏览器自动化性能门禁（冷启动、2k 块滚动 FPS、打字延迟、50 次切页内存斜率）
just bench-web --base http://127.0.0.1:18090 --email <email> --password <password>

# ④ 服务端真实协议 WebSocket 协作压测（热点房间 + 多房间并发 + 断网重连风暴 + /metrics）
just bench-ws --base http://127.0.0.1:18088 --email <email> --password <password>
```

## 基线归档规范

重大内核/渲染/同步优化或发版前，将实测结果归档至 `benchmarks/baselines/YYYY-MM-DD-<topic>.md`（必要时附 `.ndjson`），并在 [docs/README.md](../docs/README.md) 的 `performance` 索引中登记。
