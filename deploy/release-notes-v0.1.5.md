**Mica v0.1.5** — 云端数据安全 + 断线自愈 + 本地 Markdown 导入 + 编辑器打磨。

自 v0.1.4 起的全部变化(v0.1.4 之后合并的工作首次随此版本上线)。

## 可靠性 / 数据安全(M-R 里程碑)

- **云端会话四类丢数据面封死 + 完整性熔断**:
  - 坏 remote update 不再静默跳过(不越 cursor,自愈 re-bootstrap,封顶后熔断);
  - 崩溃 / 硬关闭恢复未推送编辑(未 ack 队列按 id 标记 + 持久化 + 重启重放);
  - 切页 / 关闭 / 切工作区优雅 drain,不丢在途编辑;
  - `sync.pull` 服务端分页截断不再丢尾(验证式追赶,拉到空为止);
  - 连续熔断时弹「云同步已暂停,请重试 / 刷新」提示。
- **断线自动重连**:socket 掉线 / 网络抖动自动退避重连(0.5s→30s,不引 connectivity 包),不再需要重开文档才恢复。

## 新功能

- **本地 Markdown 文件夹只读导入**(Obsidian 式):选一个 `.md` 文件夹 → 全部导入成本地文档,目录结构镜像成页树。工作区菜单 → Import → Folder。解析走权威 Rust 引擎(`export(import(x))` 定点稳定)。（决策与分档见 `docs/vault-mode.md`。）

## 修复

- **表格编辑两处崩溃**:单元格编辑浮层缺 `Material` 祖先("No Material widget found");`TextEditingController` use-after-dispose(切换单元格时)。

## 打磨

- 编辑区:正文墨色更柔(近黑 `#24292F`)、行高 1.5→1.65、段落 / 标题间距加大、标题负字距精修。
- 工作区设置从左侧内联展开改为**居中弹窗**(不再挤动页树)。
- 表格:单元格握柄从散点改为柔和圆角握柄,行高加高。

## 后端

**无后端改动。** `mica-api` 镜像仅为版本对齐重推(内容同 v0.1.4);`mica-web` 镜像**重建**(含上述所有 web 相关改进)。

## Docker

```bash
docker pull willdockerhub/mica-web:v0.1.5
docker pull willdockerhub/mica-api:v0.1.5   # 内容同 v0.1.4
```

`latest` 也指向 v0.1.5。单机栈:`deploy/docker-compose.prod.yml`。

## Assets

- `Mica-Setup-0.1.5.exe` — Windows 桌面安装包(每用户,无需管理员)
- `mica-web-v0.1.5.tar.gz` — 预构建 Flutter web bundle(静态托管,同源 API 解析)

> 后端二进制未变(同 v0.1.4),本版不再单独附 `mica-api-server` tarball。
