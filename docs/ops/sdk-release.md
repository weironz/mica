# SDK 发布手册（crates.io + npm）

本手册规范 Mica 对外公开 SDK 的版本同步、发前门禁、双端发布与线上验收流程。

> **与主应用发版分离**：本流程只发布对外消费的 Rust crate 与 npm 包，使用独立 tag `mica-md-vX.Y.Z`；Mica 服务端 / 桌面端 / Web 主应用发版（`vX.Y.Z`）请见 [release.md](release.md)，两者互不触发。

## 覆盖包与发布依赖顺序

| 产物 | 源码目录 | 目标注册表 | 线上地址 | 发布顺序 |
| --- | --- | --- | --- | --- |
| `mica-board` | `crates/mica-board` | crates.io | <https://crates.io/crates/mica-board> | **1（前置依赖）** |
| `mica-md` | `crates/md-core` | crates.io | <https://crates.io/crates/mica-md> | **2（依赖 `mica-board`）** |
| `mica-md`（JS/WASM） | `packages/mica-md-js`（基于 `crates/md-wasm` 构建） | npm | <https://www.npmjs.com/package/mica-md> | **3（与 crates 可独立或同步发）** |

- `mica-md` 的 `render-board`（被 `render` feature 包含）可选依赖 `mica-board`。只要 `crates/mica-board` 一起升版，**必须先发布 `mica-board` 并确认 crates.io 索引可见**，再发布 `mica-md`。
- `crates/md-core` 的打包白名单（`include`）包含 `src/**/*`、`build.rs`、`README.md`、`SCHEMA.md`、`CHANGELOG.md`、`THIRD_PARTY.md` 与 4 个文档示例，不包含测试 fixtures。

## 版本号与同步清单

破坏性变更判定以 [crates/md-core/CHANGELOG.md](../../crates/md-core/CHANGELOG.md) 为准（公开 API 签名、`SCHEMA.md` 项移除/改名、`serialize` 字节级输出变化、`SCHEMA_VERSION` 递增）。

| 文件 | 何时修改 | 说明 |
| --- | --- | --- |
| `crates/md-core/Cargo.toml` | 每次发布 `mica-md` | 更新 `version`；若 `mica-board` minor 变化，同步 `mica-board = { version = "...", path = "../mica-board", optional = true }` |
| `crates/md-core/CHANGELOG.md` | 每次发布 `mica-md` | 将 `[Unreleased]` 整理为 `[X.Y.Z] — YYYY-MM-DD` |
| `packages/mica-md-js/package.json` | 每次发布 npm `mica-md` | 与 `mica-md` crate 保持相同 `version`（无 scope） |
| `crates/mica-board/Cargo.toml` | 仅 `mica-board` 有改动需发新版时 | 更新 `version` |
| `crates/markdown/Cargo.toml` | 仅 `mica-md` minor/major 变化时 | 同步 `mica-md = { version = "0.X", path = "../md-core" }` |
| `Cargo.lock` 与 `clients/mica_rust/Cargo.lock` | 任一 crate `version` 变化时 | 本地 workspace 与独立客户端 workspace 均通过 `path` 引用上述 crate，需同步锁文件中的版本号 |

## 前置环境与凭据安全

### 工具链要求

- **Rust**：由仓库根 [rust-toolchain.toml](../../rust-toolchain.toml) 自动固定，且需安装 `wasm32-unknown-unknown` target。
- **wasm-bindgen-cli**：版本必须与 `Cargo.lock` 中 `wasm-bindgen` 精确一致（由 [scripts/check-md-wasm-toolchain.sh](../../scripts/check-md-wasm-toolchain.sh) 校验）。
- **Node.js ≥ 20 与 npm**：npm 发布统一使用 `npm publish`（`pnpm` 可用于安装依赖或 `pnpm adduser`，但不要用 `pnpm publish`，详见「实操避坑」）。
- **jq**：`scripts/build-md-npm.sh` 读取 `cargo metadata` 使用。

### 环境变量与红线

- `CARGO_REGISTRY_TOKEN`：crates.io 发布令牌（供 `cargo publish` 使用）。
- `NPM_TOKEN`：npm Granular Access Token，需具备 `mica-md` 包读写权限。

> **安全红线**：Token 只允许保存在个人系统环境变量中并在发布命令运行时读取到内存；**严禁**写入仓库文件、提交历史、CI 日志或长期保留在 `~/.npmrc`。执行 `npm publish` 时使用临时 `--userconfig` 文件，并在 `finally` / `trap` 中立即删除。

## 标准发布流程

### Step 1：同步版本号与 CHANGELOG

1. 按上表更新目标 `Cargo.toml`、`packages/mica-md-js/package.json` 与 `crates/md-core/CHANGELOG.md`。
2. 刷新两份锁文件（在仓库根目录执行）：

```bash
cargo check -p mica-md -p mica-board
cargo check --manifest-path clients/mica_rust/Cargo.toml -p mica-rust
```

### Step 2：运行发前门禁与双端打包校验

在 Git Bash（Windows）或 bash 下执行全套就绪检查：

```bash
just sdk-check
```

该命令会依次校验（脚本见 [scripts/sdk-check.sh](../../scripts/sdk-check.sh)）：

1. `cargo package -p mica-md --allow-dirty --list` 严格匹配打包白名单；
2. `RUSTDOCFLAGS="-D warnings" cargo doc -p mica-md --no-deps --all-features` 零警告；
3. 三个守门测试通过：`public_api`、`schema_doc_sync`、`never_panic`，以及 `--no-default-features` 全量测试；
4. 四个示例全部实际运行成功；
5. 调用 [scripts/build-md-npm.sh](../../scripts/build-md-npm.sh) 生成 `packages/mica-md-js/dist/{web,node}`，运行 `node --test packages/mica-md-js/test/basic.test.mjs` 与 `npm pack --dry-run`。

如本次同时发布 `mica-board`，补充执行：

```bash
cargo test -p mica-board --all-features
cargo package -p mica-board --list
cargo publish -p mica-board --dry-run
```

> 若本次未改动 `mica-board`（crates.io 已有对应兼容版本），也可在此时直接运行 `cargo publish -p mica-md --dry-run` 预检打包。若本次包含新版 `mica-board`，需在 Step 4.1 发布完 `mica-board` 后再打包/发布 `mica-md`。

### Step 3：提交版本改动并打 Tag

SDK 使用独立前缀 `mica-md-vX.Y.Z`（区别于 Mica 主应用 `vX.Y.Z`），在干净工作区提交并推送：

```bash
git add -A
git commit -m "chore(sdk): release mica-md X.Y.Z"
git tag mica-md-vX.Y.Z
git push origin main mica-md-vX.Y.Z
```

### Step 4：发布到 crates.io

确保环境变量 `CARGO_REGISTRY_TOKEN` 已加载（当前 shell 可直接读取）：

1. **先发布 `mica-board`**（仅本次 `mica-board` 升版时需要）：

```bash
cargo publish -p mica-board --locked
```

等待约 10–30 秒并确认 crates.io 索引已出现新版本：

```bash
curl -fsS https://crates.io/api/v1/crates/mica-board | jq -r '.crate.max_version'
```

2. **再发布 `mica-md`**：

```bash
cargo publish -p mica-md --locked
```

### Step 5：发布到 npm（`packages/mica-md-js`）

`packages/mica-md-js/dist/` 被 `.gitignore` 忽略，发布前确认已在 Step 2 执行过 `just sdk-check`（或重跑一次 `bash scripts/build-md-npm.sh`），使 `dist/web/` 与 `dist/node/` 处于最新状态。

**Windows PowerShell**（从环境变量读取 `NPM_TOKEN`，通过临时 `--userconfig` 发布并确保删除）：

```powershell
$tmpNpmrc = Join-Path $env:TEMP ("mica-npmrc-" + [guid]::NewGuid().ToString("N"))
try {
  $token = $env:NPM_TOKEN
  if (-not $token) { throw "NPM_TOKEN is empty" }
  Set-Content -Path $tmpNpmrc -Value "//registry.npmjs.org/:_authToken=$token" -NoNewline -Encoding ASCII
  Push-Location packages/mica-md-js
  npm publish --access public --userconfig $tmpNpmrc
} finally {
  Pop-Location -ErrorAction SilentlyContinue
  Remove-Item -Path $tmpNpmrc -Force -ErrorAction SilentlyContinue
}
```

**Git Bash / Linux / macOS**：

```bash
tmp_npmrc=$(mktemp)
trap 'rm -f "$tmp_npmrc"' EXIT
printf '//registry.npmjs.org/:_authToken=%s\n' "${NPM_TOKEN:?NPM_TOKEN is required}" > "$tmp_npmrc"
(cd packages/mica-md-js && npm publish --access public --userconfig "$tmp_npmrc")
```

### Step 6：线上验收

```bash
# 1. crates.io 版本确认（按实际发布包检查）
curl -fsS https://crates.io/api/v1/crates/mica-board | jq -r '.crate.max_version'
curl -fsS https://crates.io/api/v1/crates/mica-md    | jq -r '.crate.max_version'

# 2. npm 源站版本确认（加随机参数绕过 Cloudflare max-age=300 边缘缓存）
curl -fsS "https://registry.npmjs.org/mica-md?_=$(date +%s)" | jq -r '.["dist-tags"].latest'
npm view mica-md version dist-tags
```

- 打开 <https://docs.rs/mica-md> 确认文档队列构建成功（若依赖变更或版本刚上，约数分钟后就绪）。
- 首次公开或重要版本同步将记录更新至 [docs/plans/roadmap-done.md](../plans/roadmap-done.md)。

## 实操避坑（2026-11-28 发布实测总结）

1. **`pnpm publish` 会对未开启 Bypass 2FA 的 granular token 报 `403`**：
   - 使用未勾选 Bypass 2FA 的 `NPM_TOKEN` 时，`pnpm publish` 会返回 `Two-factor authentication or granular access token with bypass 2fa enabled is required`。
   - 改用 `npm publish --access public` 即可正常发布（如需交互式登录，也可用 `pnpm adduser --registry https://registry.npmjs.org` 走浏览器授权）。
2. **npm Staged Publishing 与 Cloudflare `max-age=300` 缓存**：
   - 新版发布后的前几分钟，直接请求 `https://registry.npmjs.org/mica-md` 可能命中 CDN 缓存而暂时看到 `0.0.0-stage` 或旧缓存。
   - 请通过 `?_=<时间戳>` 查询参数绕过边缘缓存确认 `dist-tags.latest`，切勿误判失败并重复执行 `npm publish`。
3. **干净目录直接 `npm publish` 会发空包**：
   - `packages/mica-md-js/dist/{web,node}` 不入 Git。任何时候切换分支或清理工作区后，必须先执行 `just sdk-check`（或 `bash scripts/build-md-npm.sh`），再执行 `npm publish`。
4. **不要同时在 `Cargo.toml` 写相同的 `repository` 和 `homepage`**：
   - 同 URL 同时出现在两字段会触发 `cargo::redundant_homepage` 警告，仅保留 `repository`（以及 crate 文档站点 `documentation = "https://docs.rs/..."`）即可。
5. **Windows 下 `node --test <目录>` 可能静默失败**：
   - 必须传入具体测试文件路径（`node --test packages/mica-md-js/test/basic.test.mjs`）；`scripts/sdk-check.sh` 已内置该写法。

## 异常撤回与补救

- **crates.io**：不可删除版本。若刚发布的版本存在严重缺陷，立即撤回并发布补丁版本：

```bash
cargo yank --version X.Y.Z mica-md
# 如问题在 mica-board，同步 yank 并升补丁版
cargo yank --version X.Y.Z mica-board
```

- **npm**：超过 72 小时或已有下游依赖时不可 `unpublish`；优先标记废弃并发布补丁版本：

```bash
npm deprecate mica-md@X.Y.Z "Critical bug in X.Y.Z, please upgrade to X.Y.(Z+1)" --userconfig "$tmp_npmrc"
```

## CI 就绪检查入口

[.github/workflows/sdk.yml](../../.github/workflows/sdk.yml) 提供手动触发的 GitHub Actions 就绪检查（`workflow_dispatch`），在官方 Ubuntu runner 上执行与本地 `just sdk-check` 完全一致的步骤；出于凭据安全约定，**实际 `cargo publish` / `npm publish` 始终在操作人本地执行，不托管给 CI**。

## 历史记录

- **2026-10-29（S5 验收）**：`mica-md` SDK 化拆包与测试门禁落地，详见 [S5-sdk-release.md](../single-source/stages/S5-sdk-release.md)。
- **2026-11-28（0.1.0 首次公开发布）**：发布 `mica-board 0.1.0`、`mica-md 0.1.0`（tag `mica-md-v0.1.0`）与 `mica-md@0.1.0`（npm，554.1 kB）。
