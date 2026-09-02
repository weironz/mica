# 品牌资产

**`mica-logo.svg` 是唯一的源**,应用里每一处标志都由它生成 —— 没有第二份手工维护的
拷贝,除了 `web/favicon.svg`(逐字节拷贝,有测试盯着)。

> 2026-09-02 起,这个目录下不再放旧稿的位图导出(`mica-logo.png` / `mica-logo.ico`)。
> 它们是上一版「晶体」标志的产物,SVG 一换就成了错的,而放在一个写着「唯一的源」的
> 目录里,迟早有人拿它们当输入。要回看用 `git log -- assets/`。

## 它怎么变成应用里的图标

```
assets/mica-logo.svg
   │
   ├─ clients/mica_flutter/test/icon_export_test.dart   (golden 测试,每次 CI 都跑)
   │     ├─ test/icon_src/{16,24,32,48,64,128,256}.png
   │     │     └─ scripts/gen-icons.py
   │     │           ├─ clients/mica_flutter/windows/runner/resources/app_icon.ico  (任务栏 / exe)
   │     │           └─ clients/mica_flutter/assets/tray_icon.ico                   (托盘)
   │     └─ clients/mica_flutter/assets/logo_mark.png   (512,应用内 MicaLogo 画的就是它)
   │
   └─ clients/mica_flutter/web/favicon.svg   (逐字节拷贝 —— 标签页图标必须早于 Flutter 存在)
```

**每一档都是各自从矢量光栅出来的,不是把大的缩小。** 实测(2026-09-02):512 → 16 走
`FilterQuality.medium`,M 中间那个 V 形缺口会被糊掉一半;直接在 16 上光栅则保得住。
`.ico` 本来就是个容器,就该每档放各自的画。

**`logo_mark.png` 本身就是那道 golden**(512 那一帧)。所以生成器和门禁是同一次动作,
它不可能悄悄落后于 SVG —— 改了 SVG 不重新生成,CI 当场红。

## 改了 SVG 之后,跑这三条

```bash
cd clients/mica_flutter
flutter test --update-goldens test/icon_export_test.dart   # 七帧 + logo_mark.png
cd ../.. && python scripts/gen-icons.py                     # 打包两个 .ico
cp assets/mica-logo.svg clients/mica_flutter/web/favicon.svg
```

两个 `.ico` 必须一起重新生成并提交 —— 发版门禁(`scripts/release-check.sh`)会比对它们
是否逐字节相同。因为托盘图标曾经因为「改了一处忘了另一处」显示了一个多月的 Flutter
官方 logo。favicon 那条同理,由 `test/mica_logo_test.dart` 盯着。

## 这个渲染器的边界

图标链走 `flutter_svg`,它是 SVG 的**子集**:**`<filter>` / `feGaussianBlur` /
`feDropShadow` 会被静默丢弃**。上一版 SVG 靠两圈高斯模糊做核心辉光,在应用里其实一直是
硬边的 —— 浏览器里好看不等于应用里好看。层次只能用 `linearGradient` / `radialGradient`
和实体路径做。
