# Mica Performance Benchmark Suite (`mica-bench`)

独立的纯 Rust（L1 + L2）性能基准与压测套件，零额外第三方依赖，面向长期复测与回归对比。

## 目录结构

```text
benchmarks/
├── Cargo.toml        # workspace 成员（publish = false），复用主仓依赖
├── src/main.rs       # 4 大压测套件主程序
├── baselines/        # 历史 Release 基线快照（Markdown 报告与 NDJSON 原始数据）
└── README.md         # 使用说明
```

## 覆盖套件

1. **超长混排文档（默认 2,000 块 / ~302 KiB）**
   - `parse_markdown`、`session_from_markdown`（CRDT 水合）
   - 连续打字 `typing_insert_text`、Enter 分块 `split_block`、Backspace 合并 `merge_block`
   - `undo / redo`（打字撤销重做 + 结构化撤销重做）、页内查找 `in_page_find_all`、Markdown / HTML 导出
2. **CRDT 编辑轨迹回放 & 双端并发合并**
   - 内置确定性中英混排拟真轨迹（默认 20,000 步 / 重压 50,000 步），也支持通过 `--trace` 加载外部 `josephg/editing-traces`（如 `automerge-paper.json`）
   - 统计全周期 p50/p95/p99 延迟、早晚期漂移倍数、yrs 状态膨胀比、冷解码耗时、500 次双端远端合并延迟
3. **大规模合成知识库（真实磁盘 SQLite Vault）**
   - 批量入库（默认 1,000 篇 + 100 目录；重压 10,000 篇 + 1,000 目录，含双链与代码块）
   - 冷启动建树、随机单篇加载、全库搜索（高频/稀有靶点/最坏未命中）、双链图谱、SQLite 使用统计聚合
4. **大型无限白板 / 思维导图（默认 500 节点）**
   - 加载、自动布局、SVG 渲染、三方合并、全文检索文本提取

## 常用命令

```bash
# 标准基线（2k 块 / 20k 步 / 1k 篇 / 500 节点）
just bench

# 万篇重压基线（10k 篇知识库 + 50k 步 CRDT 轨迹）
just bench-10k

# 自定义规模并输出 NDJSON（便于保存至 baselines/ 或 CI 对比）
just bench --blocks 2000 --docs 1000 --json > benchmarks/baselines/latest.ndjson

# 回放外部真实编辑轨迹（josephg/editing-traces JSON 格式）
just bench --trace path/to/automerge-paper.json
```

## 归档新基线

重大内核优化或发版前，建议将实测数据与对比结论归档至 `benchmarks/baselines/YYYY-MM-DD-<topic>.md`（必要时附带 `.ndjson` 原始输出），以便跨版本追踪回归。
