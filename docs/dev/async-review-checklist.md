# 异步结果归属与资源释放：评审清单

评审任何「await 之后写共享状态」或「拿了资源要还」的改动时逐条过。这比笼统的「注意并发」
有用：下面每一条都对应一个真实出过的 bug。

## 1. 结果绑定它的归属

一个请求发出时属于谁，回来时还得属于谁，结果才能提交。

| 归属 | 什么时候变 | 用什么判定 |
| --- | --- | --- |
| 服务器 + 账号 | 退出登录、换账号、换服务器 | `SessionController.generation` + `origin()`（令牌刷新不变） |
| 工作区 | 切换工作区 | 发请求时的 `workspace.id` vs 当前 `_selectedWorkspace?.id` |
| 文档版本 | 本地又编辑了、远端又推了 | 发请求时的版本号 / 代际 vs 当前 |

- **shell 的用户操作**（`_run(() async {...})`）：`_run` 用 `runOwned` 记下归属，
  写共享状态前一行 `ensureOwner(_owner)`；涉及「选中哪个页面」再加
  `workspaceId:` / `selectedWorkspaceId:`。过期结果抛 `StaleResult`，`_run` 静默丢弃。
  （`lib/session/request_owner.dart`）
- **views 控制器的用户操作**：方法开头 `final claim = _claim();`，写 `session` 前
  `_ensureCurrent(claim);`。后台刷新走已有的 `_belongsToCurrentSession`（#148）。
- **错误路径同样要判归属**：旧账号请求的 401 不能结束新账号的会话（#147 同类）。
- **UI 门控不算判定**：「忙的时候不让退出登录」（`_isBusy`）挡住的只是一条路径，
  后台 401、换服务器照样能在请求途中换掉会话。判定必须在提交点。

## 2. 跨 await 之后先查，再写

- `await` 之后第一件事：`mounted`（或控制器的 `isMounted()`）+ 上面的归属。
- `BuildContext` 跨 await 由 `use_build_context_synchronously` lint 管（flutter_lints 默认开）。
- 被打断的请求（同一个 key 又发了新请求）用请求序号丢弃旧回答（`TreeRequestSeq`）。

## 3. 延迟执行的闭包在入队时捕获数据

`addPostFrameCallback` / `Timer` / `Future.delayed` 的闭包**执行时**才读字段，
读到的是那时的选中页、那时的会话。入队时把需要的值拷进局部变量，执行时比对还是不是它。

## 4. 资源一律走 finally / RAII

- Dart：文件句柄、临时目录、`http.Client`、订阅、计时器 —— `try/finally` 或 `dispose`
  里成对释放（范例：`updater_desktop.dart` `_downloadInstaller`，任何失败都删临时目录）。
- Rust：锁和连接靠作用域释放；**事务里不做网络 I/O**（配额锁的约定见
  `app-core/src/store.rs` `lock_file_quota_owner`）。
- 释放顺序是承重的就写测试钉住（图片 dispose 时序，见 [lessons §5](lessons.md)）。

## 刻意没做的

不做通用的「请求租约」框架：各处归属的形态不同（账号、工作区、文档版本、请求序号），
硬抽象只会让每处都多一层间接。机制只有两个小件（`request_owner.dart`、控制器的
`_claim`），其余靠这份清单在评审时逐条问。
