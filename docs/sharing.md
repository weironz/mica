# 页面分享(Publish-to-web)

把一篇云端文档发布成**公开只读链接** `https://your-server.example.com/s/{token}`,
任何人无需登录、无需装 Mica 即可在浏览器里读到。v0.5.6 起。

**只云端有**:分享依赖服务端渲染 + 集中式 token 表,本地/离线工作区没有这个入口
(和版本历史同样的 cloud-only 门)。

## 设计取舍(参照 Outline,非 AFFiNE/AppFlowy)

调研了 Outline / Notion / AFFiNE / AppFlowy 的公开分享,最终抄 **Outline 那套**,
因为它在"和我们相同约束"下最干净:

1. **token 即能力,不是 doc id**。公开 URL 里放的是一段不可猜的随机 token
   (~128 bit,两个 uuid simple 拼接),**不是** document 的真实 id。AFFiNE / AppFlowy
   的公开分享把原始 doc id 暴露在 URL 里、只靠权限层兜底 —— 那是它们最弱的一环
   (id 可枚举、可关联)。token 和行 id 分开存,链接可轮换而不动数据行,token 也不会
   跟着行 id 出现在任何日志里。
2. **软撤销 + 每次读都复查**。撤销是写 `revoked_at` 时间戳,不是删行。公开读路径每次都
   `WHERE revoked_at IS NULL`,**没有"这个 token 有效"的缓存**,所以撤销**即时生效**
   (下一次访问就是 404),还留了审计痕迹、日后能重新签发新链接。
3. **不可区分的 404**。坏 token、已撤销、文档不存在 —— 一律返回同一个朴素 404 页,
   不泄露"这个 token 存在但没权限"之类信息。
4. **默认 noindex**。分享页默认带 `<meta name="robots" content="noindex">`,不被搜索
   引擎悄悄收录;`allow_indexing` 是显式 opt-in(schema 已留,MVP 未在 UI 暴露)。
5. **复用服务端 HTML 渲染 + 能力 URL blob**,不推 SPA、不把 CRDT 文档下发给匿名客户端。
   分享页就是一坨服务端渲染好的静态 HTML(复用导出用的 `export_html`)。

## 数据模型

`migrations/0008_document_shares.sql`:

```
document_shares(
  id uuid PK,
  token text UNIQUE,           -- 公开能力,和 id 分开
  workspace_id uuid FK CASCADE,
  document_id  uuid FK CASCADE,
  created_by   uuid FK RESTRICT,
  created_at, revoked_at,       -- revoked_at NULL = 活跃
  allow_indexing   bool = false,
  include_children bool = false -- 子树分享:schema 已留,MVP 不做
)
```

- **一文档一活跃分享**:`CREATE UNIQUE INDEX ... (document_id) WHERE revoked_at IS NULL`
  —— 部分唯一索引,不用 EXCLUDE 约束(免 btree_gist 扩展)。再次分享返回同一行(幂等);
  撤销后才能签发新 token。
- **按 token 查**:`CREATE INDEX ... (token) WHERE revoked_at IS NULL`,公开读路径走它。
- `include_children` = 子树分享的地基(Notion 头号"过度分享"footgun、Outline 最易出 bug
  的路径),MVP 存 false 且不暴露,子树成员校验以后再做。

## 服务端

**store**(`crates/app-core/src/store.rs`,全用静态 SQL 字面量,不用 `format!` 拼串,
过 sqlx 的动态 SQL lint):

| 函数 | 作用 |
| --- | --- |
| `fetch_active_share_for_doc` | 查某文档当前活跃分享(给管理端点看状态) |
| `create_or_get_share` | findOrCreate:`ON CONFLICT DO NOTHING` + 回查,天然幂等 |
| `revoke_share` | 软撤销(写 `revoked_at`) |
| `fetch_share_by_token` | 公开读路径按 token 查活跃行 |

**管理端点**(`crates/api-server/src/routes/documents.rs`,挂在 `/api` 下,带 auth +
`ensure_workspace_editor`):

```
GET    /api/workspaces/{ws}/documents/{id}/share   -> {shared, token}
POST   /api/workspaces/{ws}/documents/{id}/share   -> {shared:true, token}   幂等
DELETE /api/workspaces/{ws}/documents/{id}/share   -> {shared:false, token:null}
```

**公开页**(`public_share_page`,挂在 `share_router()` 里,**在 `/api` 之外**,永远见不到
auth guard;nginx 把 `/s/` 反代到后端):

```
GET /s/{token}   (无 auth)
```

流程:`fetch_share_by_token` → `store::current_payload`(读文档当前快照)→
`inline_blob_hrefs` → `export_html`(复用导出的 fragment 渲染)→ `render_share_shell`
包成独立 HTML(带 title / noindex / 内联 CSS / "用 Mica 制作"页脚)。任何一步失败都走
同一个 `not_found()` 闭包(不可区分 404)。

### `inline_blob_hrefs` —— 顺手修的图片 bug

`export_html` 读 block 的 `url` 字段渲染 `<img>`,但**上传的图片**在 block 里只存
`{file_id, name}`、没有 `url` → 导出/分享页里 `<img src="">` 空图。

`inline_blob_hrefs(blocks, workspace_id)` 在渲染前把这类 block 的 `url` 填成
`blob_href(ws, file_id, name)` = `/api/workspaces/{ws}/files/{file_id}/blob/{name}`。
这个 blob 端点是**公开能力 URL**(`is_blob_path`,file_id 本身即凭证,见 `tests/blob_public.rs`),
所以在**未登录的分享页上也能取到图**。导出 HTML 一并受益(不再空 src)。

## 客户端(Flutter,cloud-only)

- `lib/api/client.dart`:`getShare` → `({bool shared, String? token})`、`createShare` → token、
  `deleteShare`。
- `lib/ui/dialogs.dart` `_ShareDialog`:一个 `SwitchListTile`(公开开关)+ `SelectableText`
  展示 URL + 一键复制(`Clipboard.setData`)。
- `lib/main.dart`:页面菜单加「分享」项(`Icons.public`),`onShare = local ? null : _openShare`
  —— 本地工作区灰掉。URL 由客户端拼:`buildUrl: (token) => '${_api.baseUri.origin}/s/$token'`。

## MCP

薄代理(`crates/mcp-server`)加两个工具:

| 工具 | 作用 |
| --- | --- |
| `mica_share` | 发布成公开只读链接,返回可直接打开的完整 URL(`{base}/s/{token}`);幂等,再分享返回同一个 |
| `mica_unshare` | 关闭公开链接(立即 404) |

## 部署

`deploy/nginx.conf`:在 SPA fallback **之前**加 `location /s/ { proxy_pass http://api:8080; ... }`,
否则 `/s/{token}` 会被前端路由吞掉。

## 安全守则(8 条)

token 不可猜且与 doc id 解耦、公开路由零 auth 但只能读活跃分享、撤销即时、404 不可区分、
默认 noindex、匿名端拿不到 CRDT 只拿渲染好的 HTML、blob 走既有公开能力 URL(不新开权限面)、
子树分享留 schema 但 MVP 不启用(避免过度分享)。

## 端到端验证(prod v0.5.6)

创建分享→拿到 64 位 token;`GET /s/{token}` 无 auth → HTTP 200,渲染
`<!doctype>` + `<title>` + `noindex` + 页脚;坏 token → 404;撤销后 → 立即 404;
幂等再分享 → 返回同 token。**全过**。(带图分享页的图片实时加载因当时 CN→prod 网络抖动
未补抓,但 `inline_blob_hrefs` 逻辑 + blob 公开路径此前已实测 200。)
