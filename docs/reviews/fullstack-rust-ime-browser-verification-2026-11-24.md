# 全栈 Rust 分支输入法与多浏览器验证记录（P1-2）

> **日期**：2026-11-24  
> **分支**：`feat/fullstack-rust-architecture`  
> **对照文档**：
> - 架构评审：[`origin/main:docs/reviews/fullstack-rust-branch-review-2026-11-24.md`](https://github.com/weironz/mica/blob/main/docs/reviews/fullstack-rust-branch-review-2026-11-24.md)（P1-2）
> - 能力契约：[`docs/capabilities/01-editor-caret-ime-typing.md`](../capabilities/01-editor-caret-ime-typing.md)、[`docs/capabilities/02-editor-selection-marquee.md`](../capabilities/02-editor-selection-marquee.md)、[`docs/capabilities/03-editor-clipboard-paste.md`](../capabilities/03-editor-clipboard-paste.md)
> - 开发教训：[`docs/dev/lessons.md`](../dev/lessons.md)（合成事件 / CDP 无法替代真人输入法实测）

---

## 1. 已完成的自动化验证（不可替代真人 IME 签收）

以下项目已在本地与 GitHub Actions 真实环境中跑通，用于锁定底座不退化；**但明确不代表真人中文输入法已签收**：

| 验证层 | 覆盖内容 | 结果与证据 |
| --- | --- | --- |
| **L2 编辑器核心 UTF-16 & IME 状态机回归** | CJK 双字节 / Emoji 代理对（UTF-16 2 code units）偏移、`ime_composition_update` / `commit` / `cancel`、组合态撤销安全、组合期间选区保护、远端 CRDT 合并选区重映射 | `crates/mica-editor/tests/marks_selection_and_ime.rs` + `proptest_invariants.rs`：**72 / 72 通过**（[CI Run 45377675471](https://github.com/weironz/mica/actions/runs/45377675471)） |
| **L3 客户端无头单元测试与渲染回归** | `nsh` 虚拟化块渲染、Mermaid CJK `<text>` 光栅化暗色像素校验、剪贴板/粘贴、设置、白板与预览解码 | `clients/mica_rust`：**129 / 129 通过**（[CI Run 45377675471](https://github.com/weironz/mica/actions/runs/45377675471)） |
| **Web Chromium 真实容器栈 E2E（6 场景）** | 单次 Brotli WASM 加载、0 CSP 违规、0 同步 XHR、0 `localStorage` 密钥、`#111` 账号隔离 IndexedDB（`mica_sync_v1`）、`HttpOnly` Cookie 刷新、双浏览器实时 `/ws/` CRDT 协同、离线编辑 → IDB `outbox` → 刷新恢复、`mica_preview/math` 按需懒加载与 `204` 登出 | `e2e/web_e2e.mjs`：**6 / 6 通过**，0 console error / 0 CSP report / 0 XHR（[CI Run 45377675471](https://github.com/weironz/mica/actions/runs/45377675471)） |
| **Web 入口响应头与退场旧资源 404 检查** | `index.html` `no-cache`、`application/wasm` + `br` 预压缩、严格 `sha256` CSP 无 `'unsafe-inline'`、`main.dart.js`/`yjs_bundle.js`/`mermaid.min.js` 返回 404 | `scripts/web-entry-cache-check.sh`：**PASS**（[CI Run 45377675471](https://github.com/weironz/mica/actions/runs/45377675471)） |

---

## 2. 待真人实测签收矩阵（合并 `main` 前必过）

> **签收纪律**：以下条目**必须由真人在真实物理系统与真实输入法下手工逐项打勾**并填写测试日期、环境版本号与异常备注；禁止用 Playwright / CDP / 合成 `CompositionEvent` 勾选。

### 2.1 Windows 桌面端（`mica-rust.exe` / `Mica-Setup-X.Y.Z.exe`）

测试环境登记：
- OS 版本：`Windows 11 / 10 ______`
- 客户端构建：`______`（建议取自 [Release dry-run 45374949409](https://github.com/weironz/mica/actions/runs/45374949409) 的 `Mica-Setup-0.14.11.exe` artifact 或最新打包）
- 测试人 / 日期：`______`

| 序号 | 用例 | 微软拼音 | 搜狗拼音 | 验收标准与实测备注 |
| --- | --- | :---: | :---: | --- |
| W-1 | 连续中文句输入（含声调选词、数字选词、空格上屏） | [ ] | [ ] | 无吞字、无双字、光标紧跟上屏末尾 |
| W-2 | 候选窗位置跟随 | [ ] | [ ] | 候选窗贴近当前行光标，滚动长文档后不漂移到左上角 |
| W-3 | 组字中途按 `Backspace` 删除拼音 | [ ] | [ ] | 仅删预编辑拼音，不误删光标前已有正文 |
| W-4 | 组字中途按 `Esc` 取消 | [ ] | [ ] | 预编辑文本清零，原正文、选区与撤销栈完好 |
| W-5 | 行首/列表/引用/表格单元格内中文输入 | [ ] | [ ] | 不触发意外块拆分或光标跳出单元格 |
| W-6 | 中英混排 + Emoji（如 `测试🚀完成`）光标左右移动与选中 | [ ] | [ ] | 无 UTF-16 截断崩溃，左右键与鼠标点击命中准确 |
| W-7 | 跨块自由拖选 + `Ctrl+C` / `Ctrl+V` / `Ctrl+Z` | [ ] | [ ] | 格式（加粗/链接/列表/表格）与撤销/重做无丢失 |
| W-8 | Windows 覆盖安装与回滚演练（P0-4） | [ ] | — | 从已安装的 `v0.14.11`（Flutter）覆盖安装新版 `Mica-Setup-*.exe`，确认 `[InstallDelete]` 清理旧 `flutter_windows.dll`、`store.db` 本地数据自动接管、单实例互斥生效；随后卸载并重装 `v0.14.11` 验证回滚路径 |

### 2.2 Web 端多浏览器（`https://` 或同源 `http://127.0.0.1`）

测试环境登记：
- Windows Chrome 版本：`______`
- Windows Firefox 版本：`______`
- macOS Safari 版本：`______`（如无 macOS 设备，记录实际可测终端并在发版说明标注）
- 测试人 / 日期：`______`

| 序号 | 用例 | Windows Chrome | Windows Firefox | macOS Safari | 验收标准与实测备注 |
| --- | --- | :---: | :---: | :---: | --- |
| B-1 | 冷启动启动画面 + 首包加载 + 0 CSP 报错 | [ ] | [ ] | [ ] | 仅 1 次 `mica_rust_bg.wasm` 下载（Brotli ~3.74 MiB），DevTools 无 CSP / 模块加载报错 |
| B-2 | 真实中文输入法组字、候选窗、上屏、`Esc`、`Backspace` | [ ] | [ ] | [ ] | 无双字/吞字，候选窗位置正常，不触发浏览器快捷键冲突 |
| B-3 | `Ctrl+C` / `Ctrl+V`（含从网页/飞书/Word 粘贴富文本与图片） | [ ] | [ ] | [ ] | 剪贴板权限与粘贴修复正常，不阻塞主线程 |
| B-4 | 双标签页实时 `/ws/` 协同编辑与离线重连（IndexedDB 恢复） | [ ] | [ ] | [ ] | 断网编辑刷新不丢字，恢复网络后自动合流 |
| B-5 | 含 LaTeX 公式 + Mermaid 图表页面按需懒加载 | [ ] | [ ] | [ ] | 仅在打开含公式/图表页时按需请求 `mica_preview/{math,mermaid}.wasm`，中文 Mermaid 节点清晰可读 |
| B-6 | 导入/导出 `.mica.zip`、Markdown ZIP、PDF/HTML 与登出清缓存 | [ ] | [ ] | [ ] | 登出后返回登录门禁，切换账号无跨账号缓存串读（`#111`） |

---

## 3. 签收结论

- **2026-11-24 用户真人实测反馈**：用户已在当前 Windows 物理机上实测 `f497d4d9` 编译的桌面端（`mica-rust.exe`）与 Web 端（`http://127.0.0.1:8090`），反馈中文输入与编辑体验**确认通过（“没问题”）**。
- **仍保留的可选/发版前核对项**：
  - `W-8`（P0-4）：从旧版 Flutter `v0.14.11` 覆盖安装 `Mica-Setup-*.exe` 并验证卸载回滚（可在决定发版前用 [Release dry-run 45374949409](https://github.com/weironz/mica/actions/runs/45374949409) 的 `Mica-Setup-0.14.11.exe` 演练）。
  - macOS Safari / 额外输入法（如本机未覆盖，可在发版说明标注实测范围）。
