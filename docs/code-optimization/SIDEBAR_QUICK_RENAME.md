# 目录树页面双击快速重命名

日期：2026-10-08。开发基线 v0.13.48。

## 行为

- 可编辑页面行：第一次鼠标单击立即打开页面，第二次在双击时间与位置范围内的有效点击打开现有重命名弹窗。
- 弹窗预选当前名称；直接输入替换，Enter或保存按钮提交，Esc或取消退出。空白、未变化名称不提交。
- Ctrl/Shift/Cmd点击仍执行多选；只读页面不提供双击改名。文件夹展开、触屏点击与滑动保持原有行为。
- 拖拽、取消手势、切换到另一页面和进入行内编辑都会清除点击配对，避免把不连续的点击误认成双击。

## 实现边界

`lib/ui/widgets.dart` 的 `DocumentListItem` 在已有 `onTap` 上判断第二次鼠标点击，时间来自原始指针事件，阈值采用Flutter手势常量；不引入会延迟每次单击的 `DoubleTapGestureRecognizer`，不设置额外定时器。

第二次点击调用已有 `onRename`，由 `main.dart` 的 `_promptRenameView` 打开同一弹窗，继续使用 `_commitRename` 和既有本地/云端保存回调。没有新增依赖或修改服务端/数据模型。

## 验证

- 相关Flutter测试 **53项通过**，包括新增14项，覆盖真实WorkspaceView→弹窗→保存回调、完整名称预选、取消/空白/未改动、多选、只读、触屏、时间/位置范围、拖拽和A→B→A。
- 原有单击即时响应测试改用鼠标事件，仍要求不推进时间就完成打开。
- 新测试使用测试时钟生成非零且递增的指针时间戳：`tester.tap`默认时间戳为零，不能证明双击时间判定有效。
- 临时移除第二次点击的重命名入口，真实弹窗用例实测失败；恢复后通过。
- Flutter分析：无新增错误/警告，仅136项既有info。
- Playwright真实鼠标/键盘覆盖单击打开、双击只打开一次并弹窗、Enter保存一次、Esc取消、Ctrl多选和跨页面点击配对。使用真实WorkspaceView/弹窗与合成文档、回调，不访问生产数据。语义树只用于取得标签/截图；关闭其鼠标命中，让物理点击进入画布，避免无指针时间戳的语义tap代替鼠标输入。
- CI增加同一浏览器回归，归档截图与结果。
- 提交 `f0b6c29` 的 [主CI](https://github.com/weironz/mica/actions/runs/37721419764) 全部通过，新增浏览器用例与本机结果一致。
- 同一提交的 [Windows离线/联网集成测试](https://github.com/weironz/mica/actions/runs/37721419904) 两组全部通过。

[弹窗截图](assets/sidebar-rename-dialog.png)、[保存后截图](assets/sidebar-rename-saved.png)、[浏览器结果](assets/sidebar-rename-results.json)。

复跑：

```powershell
cd clients/mica_flutter
flutter test test/sidebar_double_click_rename_test.dart test/document_list_item_test.dart test/navigation_touch_scroll_test.dart test/page_title_field_test.dart test/page_title_widget_sync_test.dart test/rename_test.dart
flutter analyze --no-fatal-infos
flutter build web --release --no-web-resources-cdn --no-wasm-dry-run --target=e2e/sidebar_rename_harness.dart --output=build/sidebar_rename --dart-define=MICA_DEV_AUTOLOGIN=false
cd ../..
node e2e/sidebar_rename_e2e.mjs
```

本次开发未发版或部署生产。
