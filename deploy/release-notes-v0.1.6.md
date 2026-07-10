# Mica v0.1.6

桌面端体验修复与自助更新。

## 新增
- **自助更新**:设置 → About → **检查更新**。发现新版一键下载并自动关闭、重启完成更新(GitHub Releases + Inno 静默安装)。*此版起生效——从 v0.1.6 往后可自动更新。*

## 字体
- **中文不再发"虚"**:桌面改用系统中文字体(Windows 微软雅黑 / macOS 苹方 / Linux Noto CJK);web 打包 **Noto Sans SC** 替代旧的 DroidSansFallback。

## 修复
- 桌面粘贴富文本时,顶部不再混入 `Version:0.9 StartHTML:… StartFragment:…`(Windows CF_HTML 描述头)。
- mermaid 图不再出现黑色背景(渲染时把画布色实体化为不透明背景)。

## 其它(服务端,已在线上)
- Personal Access Tokens(带 scope、可吊销),web/桌面设置里可自助签发。
- 定时备份 docker 化(mica-cli backup → 阿里云 OSS,加密去重增量)。

---
Windows 安装包:`Mica-Setup-0.1.6.exe`(附于本 release)。首次运行的 SmartScreen 提示 → 更多信息 → 仍要运行(未签名)。
