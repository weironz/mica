# 品牌资产

**`mica-logo.svg` 是唯一的源。** 其余文件是导出时的副产品,不要拿它们当输入 ——
两个都有会静默降级的缺陷:

| 文件 | 规格 | 为什么不能当源 |
| --- | --- | --- |
| `mica-logo.svg` | 1024×1024,透明底 | ✅ **源** |
| `mica-logo.png` | 1254×1254,**RGB(无 alpha)** | 图标会带一块实心方底 |
| `mica-logo.ico` | **只有 16×16 一帧** | 应用图标需要 7 帧(16→256);用这个,任务栏 48 和 Alt-Tab 256 都是把 16px 放大的糊图 |

## 它怎么变成应用里的图标

```
assets/mica-logo.svg
        │  clients/mica_flutter/test/icon_export_test.dart   (MICA_EXPORT_ICONS=1)
        ▼
clients/mica_flutter/test/icon_src/{16,24,32,48,64,128,256}.png
        │  scripts/gen-icons.py
        ▼
clients/mica_flutter/windows/runner/resources/app_icon.ico   (任务栏 / exe)
clients/mica_flutter/assets/tray_icon.ico                    (托盘)
```

外加 `clients/mica_flutter/assets/logo_crystal.png`(512),给应用内 ≥32px 的位置用。

**16 和 24 那两帧不是这个 SVG 渲染的**,是 `MicaLogoPainter` 画的线框简化版 ——
晶体在 16px 上只剩一颗没有轮廓的蓝点(实测,不是猜的)。分级阈值 32px 写在
`icon_export_test.dart` 里,连同理由。

**改了这个 SVG,两个 `.ico` 必须一起重新生成并提交** —— 发版门禁
(`scripts/release-check.sh`)会比对它们是否逐字节相同,因为托盘图标曾经因为
"改了一处忘了另一处"显示了一个多月的 Flutter 官方 logo。
