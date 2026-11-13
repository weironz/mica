# 第 12 节评审跟进证据

结论与口径见施工文档第 13 节。图像均为真实 Windows 客户区原图，未缩放保存；`1440` / `900` 是外窗宽度，客户区分别为 1424×961 / 884×761，DPI 96、引擎 DPR 1。

- `zh-*` / `en-*`：设置页，中英文、浅深色、宽窄窗口共 8 张。
- `settings-metrics.json`：三组分段选择的背景边界实测；`native-test-output.txt` 是原生测试输出，`native-settings-test.dart` 是临时验收入口，复制到 Flutter 项目的 `integration_test/` 后使用现有 `test/support/native_build.dart` 校验 FFI 指纹。它不属于产品入口。
- `runtime-source-hashes.json`：Windows 源码 SHA-256，255 个已跟踪运行源码逐一与本地匹配；`source-provenance.json` 记录基线、修改文件、工具链与测量覆层哈希。
- `startup-runs.json` / `startup-summary.json`：40 个独立进程的原始记录与汇总，P95 使用最近秩；`startup-environment.json` 记录机器环境。
- `before-bundle-manifest.json` / `after-bundle-manifest.json`：计时用 Release 产物的大小与 SHA-256，基线不含 Noto、D 包含 Noto。
- `ci-summary.json`：完整本地 CI 的实际通过数；Windows 原生补验单独记录，不冒充本地 CI 自带检查。

复算汇总：

```bash
python3 docs/plans/assets/ui-refresh/review-followup/summarize.py docs/plans/assets/ui-refresh/review-followup
```

重新测量时，在隔离源码中运行 `instrument.py <Flutter项目目录> <输出清单.json>`；脚本使用同目录的 `startup_probe.dart`。两份构建都清除旧 Flutter AOT / 资源输出，并使用来源记录中的相同构建标志。`measure-startup.ps1 -Before <基线exe> -After <当前exe> -Output <结果目录>` 依靠现有原生首帧回执及临时 Dart 计时结果，各执行 20 次；从输出目录取回 `runs.json`，归档为 `startup-runs.json` 后可复算。

测量脚本会为每次启动创建独立配置目录，发现已有 Mica 进程就停止，不覆盖安装或关闭用户程序。OS 磁盘缓存不清除；这些结果不能代表重启后的磁盘冷启动或低配机器。
