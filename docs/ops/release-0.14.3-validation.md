# 0.14.3 发布与验收

日期：2026-11-01。发布标签 `v0.14.3` 指向 `a660ac95`（`chore(release): 0.14.3`）。
这是 0.14.2 之后的补丁版：Web 文档核心改用 Rust `mica-core` 的 wasm，退役 yjs bundle；
新建页面输入首行不再先留一个空段。无新 SQL 迁移。

**当前结论：发行物已公开、生产版本已更新；验收尚有未通过项，不能标为全绿。**
生产 `/yjs_bundle.js` 返回 200 的 SPA HTML，要求是 404。修复和防回归门禁在 main 的后续提交中，
不属于已公开的 `v0.14.3`；本次没有覆盖旧 tag 或镜像。需通过后续补丁版交付。

## 门禁与发行物

发布前实际回执（均核对日志末尾成功标记及退出码）：

- `just release 0.14.3` 的 `release-check passed for 0.14.3`、`EXIT=0` 已核实。
  Rust workspace `receipt: 1177 passed`，DB 测试连接本地测试 Postgres；
  Flutter 单元 `receipt: 2084 passed`，analyze 无错误/警告；Clippy `-D warnings` 通过。
- 原发布脚本只构建 Markdown wasm，导致文档 wasm 的 freshness 测试因无产物而跳过。
  该次总门禁不能证明文档 wasm 已验；随后补建文档 wasm并实际运行 freshness 测试，5 项通过。
- `bash scripts/browser-tests.sh` 实际发现 **9 套、41 项通过**，包含用户要求的 8 套及新增 benchmark：
  `image_clipboard_web` 3、`markdown_codec_contract_web` 12、`markdown_codec_unavailable_web` 1、
  `mermaid_preview_web_async` 1、`prefs_web` 1、`web_doc_mirror` 4、`web_doc_replica` 6、
  `web_idb_doc_store` 12、`web_replica_bench` 1。各套都有独立 receipt，脚本退出 0。
- 额外检查：`just check-structure`、`cargo fmt --all --check`、Dart Markdown 单一实现守卫及自测通过；
  `mica-core --features store` 217 项、独立桌面 FFI crate 47 项通过，日志到达 `EXTRA ALL GREEN`。
- 两套 wasm gzip 各自满足 262,144 B 硬预算：Markdown 249,708 B、文档 251,237 B。
- 本机发行物构建、上传、公开均退出 0。Windows 安装包在临时目录静默安装、首帧渲染且存活 10s，
  安装启动冒烟通过；Linux CLI 在钉死工具链的 Docker 中构建。
- GitHub Release 已公开且 `/releases/latest` 指向 `v0.14.3`，5 个资产齐全：
  `Mica-Setup-0.14.3.exe`、Windows/Linux CLI、`sha256-windows.txt`、`sha256-linux.txt`。
  发布步骤已核对每个平台的 manifest 与资产 digest；无 macOS CLI。
- 三个镜像（API / Web / CLI）均推送到 ACR 和 Docker Hub。
- `.github/workflows/` 只有手动触发入口。验收时 Linux 与 Windows 两个 self-hosted runner 都 offline；
  已对 `v0.14.3` 手动触发 [CI run 41315872469](https://github.com/weironz/mica/actions/runs/41315872469)，
  API 读回 `headSha=a660ac95…`、`status=queued`，Scope/Structure job 排队。
  **托管 CI 尚未执行，不能记为 CI 通过**。本次使用本地门禁证据。

修复后的同版本源码重新执行门禁（不重打 tag）：新建独立测试数据库、应用迁移，
`MICA_REQUIRE_DB=1`；`release-check passed for 0.14.3`，Rust 1,177 项、Flutter 2,085 项。
Flutter 比首次多 1 项，是此前文档 wasm 缺失而跳过的 freshness 检查此次实际运行。
`just check-structure` 通过（存量超长库预算警告保留）；shellcheck、入口契约自测和发布编排自测通过。
9 套浏览器测试再次取得相同的 41 项通过回执；整条重跑日志最后到达 `RETRY ALL GREEN`、退出码 0。
复审发现的 bind mount 清理和 JS MIME 假绿两项均已修正并验证。

## 部署

- `just deploy-prod 0.14.3 --check --diff` 彩排通过，随后正式部署：
  `ok=35 changed=4 unreachable=0 failed=0`。
- playbook 判定 0.14.2 → 0.14.3 无新迁移，因此没有执行迁移前还原点流程。
  本次不修改已发布迁移，不删除生产数据，不重建共享 Traefik 或覆盖其独立 slice。
- 内置验证：`/api/ready` 版本 0.14.3、首页 200、bundle digest `02ca5c3a4098…`。

## 生产只读验收

| 项目 | 结果 | 证据与范围 |
| --- | --- | --- |
| `/api/health` | 通过 | `status=ok`，`service=mica-api-server`，`version=0.14.3` |
| `/mica_doc/mica_doc.js` 与 `mica_doc_bg.wasm` | 通过 | 200、`Cache-Control: no-cache`；wasm MIME 为 `application/wasm` |
| `/mica_md/mica_md.js` 与 `mica_md_bg.wasm` | 通过 | 同上 |
| 不存在的引擎 `.wasm` | 通过 | 两个引擎前缀下均 404，没有落入 SPA |
| `/yjs_bundle.js` | **失败** | 200、返回 SPA HTML；镜像内文件已移除，但缺少 exact 404 路由 |
| `window.micaDocEngineInfo()` | 通过 | 真实 Chromium 加载生产页，返回 `crate=mica-doc-wasm`、`source_fingerprint=67311f2ad992b1c8`，无 error |
| 浏览器启动 | 通过 | v0.14.3 登录页可见；无 yjs 全局、无 console/page error |
| 生产真实编辑 | 未验证 | 实例关闭注册，无可用专用测试账号；遵守不动真实用户数据的约束，写入冒烟移到隔离栈 |

## 隔离栈功能验收

使用已发布的 v0.14.3 API/Web 镜像、本机独立 Compose 项目及测试账号。
所有功能测试写入均在测试工作区；不连接生产数据，不修改用户已安装 CLI 或 MCP 配置。
浏览器实际交互使用 Playwright；Windows 桌面的人工验收另列，不能由 Web 结果替代。

| 项目 | 结果 | 验证方式 |
| --- | --- | --- |
| 输入、重载与持久化 | 通过 | 真实 Web 输入后读取服务端 Markdown,刷新后原文仍在 |
| Ctrl+Z / Ctrl+Shift+Z | 通过 | 真实快捷键,服务端分别回到原文/恢复追加内容 |
| 两个独立 context 实时同步 | 通过 | 各自登录且不共享存储,A→B、B→A 均无需刷新;画面与 REST 正文一致 |
| `micaWebSyncTest` | 通过 | 在已加载发布 bundle 的 Chromium 调用,返回 `ok: true` |
| 新建页面首行 | 通过 | 标题 Enter 与点击正文两条路径;首行直接出现输入,bootstrap root 下只有一个 paragraph |
| 0.14.2 旧离线 IndexedDB → 0.14.3 | 通过 | 全新持久 profile,加载真实旧 bundle 后断开目标文档 socket;服务端正文/更新序号均保持不变,IDB outbox 有2条待发更新,拦截4帧;关闭旧context后再确认服务端未变。随后同profile换0.14.3,离线标记才进入服务端,刷新仍在;yjs全局已不存在 |
| 自身原生 ZIP 导入新工作区 | 通过 | 发布 API 导出,发布 CLI 导入;3/3完成无图片失败,manifest v2、native块id、树、特殊标题均保留;内链指向新view,图片70B的SHA256一致 |
| native SQLite 导入路径 | 通过(限定范围) | 现有自身导出/导入回归真跑,`receipt: 1 passed`;这只证明 Rust store 路径 |
| CLI `--json` 与服务端字段 | 通过 | 发布exe哈希等于GitHub资产;默认/with-stats两组与REST逐字段差异均为空;只有stats请求含`state_bytes` |
| 独立 MCP stdio | 通过 | 发布exe子进程连接隔离API,完成initialize、tools/list、两次mica_list_pages;未调用用户真实connector |
| 既有 Web E2E 套件 | 通过 | 19 assertions passed,0 failed;含双会话socket收敛、引擎启动/缓存头、无本源启动错误及POSIX C locale |

CLI 只在临时配置目录运行,没有覆盖正在使用的安装版和配置。原始 JSON、日志、截图不入 Git。
严格离线升级采用本次 `legacy4` 证据;前几轮中升级前数据已到服务端的结果均丢弃,不计通过。


## 验收发现的发布链缺口与修复

1. **两个示例输出名冲突**：`mica-md-wasm` / `mica-doc-wasm` 都有 `print_fingerprint`，
   Cargo 把宿主示例写到同一个 `target/debug/examples` 路径，后构建覆盖前者。
   Markdown 的 `SOURCE_FINGERPRINT` 因此可能误写成文档引擎指纹。
   已改为独立的 `print_md_fingerprint` / `print_doc_fingerprint`，Cargo metadata 确认各自只有该指纹目标。
   已发布镜像内 wasm 字节与 tag 的构建产物一致；该 stamp 不参与运行时执行，不能用它冒充旧镜像 freshness 通过。
2. **本地发布漏构建文档 wasm**：`release-check.sh` / `release-images.sh` 补上 `build-doc-wasm.sh`，
   编排自测要求两套引擎都在 Flutter 测试前构建，镜像构建另跑 `web_engine_fresh_test.dart`。
3. **Flutter 增量输出残留退役资源**：生产构建先清空 `build/web` 的内容、保留挂载目录本身；
   直接删掉目录会令运行中的 nginx bind mount 留在旧目录。Docker 构建拒绝缺失引擎和残留 yjs 文件。
4. **SPA 把退役 JS 请求伪装成 200**：prod/dev nginx 均为 `/yjs_bundle.js` 加 exact 404，
   引擎前缀下缺失资源也不再回退首页。新增 `web-entry-cache-check.sh` 检查 HTTP 契约，接入 CI，
   自测覆盖错误状态、缓存头、HTML 冒充 JS/wasm、网络失败，最终打印 `ALL GREEN`。
   本地用发布镜像挂载修复配置实测：两套引擎 200/no-cache/正确 MIME、退役 yjs 和缺失 wasm 404，
   检查脚本到达 `Web entry contract: ALL GREEN`。`Dockerfile.web` 的实际本地构建也通过。
   生产仍运行 tag 内原配置，故生产该项保持失败。

## 桌面与未覆盖范围

Windows 安装启动冒烟已通过。用户仍需在安装的 0.14.3 中确认：富文本粘贴、
本地导入/导出包、新建页面不再出现空行。此处不把浏览器测试或 Rust store 测试记为桌面人工验收。

`mica-cli` / MCP 客户端升级与 API 部署分别生效；本次没有替换用户正在使用的安装版二进制。
`mica-md` crate 和 npm 包均未发布，继续 `publish = false`。
N1 及本版完成的单一实现后续移入 `roadmap-done.md`，N2–N4 保留在 `roadmap.md`。
临时脚本、原始测试输出、截图、账号凭据均保留在仓库外，不加入提交。
