# 工作区邀请(添加成员 = 邀请,对方接受后加入)

2026-10-21 起。

## 为什么改

`POST /api/workspaces/{ws}/members` 原来按邮箱查到用户就**直接插入成员**:

1. 对方没有同意的机会 —— 被谁拉进哪个工作区,自己说了不算。
2. 未注册邮箱答 404 —— 任何工作区管理员都能拿它问「这个邮箱注册过没有」(用户枚举信道)。

## 设计(最小充分)

- 表 `workspace_invitations`(迁移 `0032`):`id`、`workspace_id`(工作区删除时级联)、
  `email`(trim + ASCII 小写,与 `users.email` 同一规则)、`role`、`invited_by`、`created_at`;
  `(workspace_id, email)` 唯一。DB 兜底两条不变量:`role <> 'owner'`、`email = lower(email)`
  (lessons §2:不变量只写在 API 里等于没写)。
- **以邮箱为键,不查用户表**:已注册与未注册邮箱走完全相同的路径、返回相同形状的响应
  (响应里没有任何用户 id)。对方以后注册并验证邮箱,就能看到这条邀请。
- 受邀方只认**未删除且邮箱已验证**(`deleted_at IS NULL AND email_verified_at IS NOT NULL`)
  的账号。未验证的邮箱证明不了谁拿着收件箱;墓碑账号(C4)两者都没有。
- **接受** = 一个事务:`DELETE … RETURNING` 认领邀请 → 插入成员
  `ON CONFLICT DO NOTHING`(已是成员时**不改角色**,避免 0028 那类「重新添加 = 降级」)。
  同一邀请重复接受:第二次找不到可认领的行 → 404,成员关系不变。
- 别人的邀请 / 已失效的邀请:接受、拒绝都答 **404**(不是 403,免得确认这个 id 存在)。
- 邀请一个**已经是成员**的邮箱 → **409**。管理员本来就看得到成员列表,所以这不泄露信息;
  若静默成功,会留下一条接受时什么也不做的「死邀请」。再次邀请同一个尚未接受的邮箱 →
  原地更新角色(一邮箱一行)。
- 权限沿用原规则:owner / admin 才能邀请、查看待接受列表、撤回;不能邀请成 owner(400)。
- 成员被移除后可以再次被邀请(接受时邀请行已删除,唯一约束不挡)。
- 不发邮件、不做实时推送:客户端在启动、登录、刷新工作区列表时拉一次 `GET /invitations`。

## 端点

全部在 `scope_guard` 之后(需要登录),不进 `is_public`,也不需要 `rate_limit` 登记(那张表只管
未登录 / 跑 Argon2 的路径)。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST | `/api/workspaces/{ws}/members` | **创建/更新邀请**(URL 不变,响应体改为 `{"invitation": …}`);已是成员 409 |
| GET | `/api/workspaces/{ws}/invitations` | 待接受列表(owner/admin) |
| DELETE | `/api/workspaces/{ws}/invitations/{id}` | 撤回,204 |
| GET | `/api/invitations` | 当前用户收到的邀请:含 `workspace_name`、`invited_by_name`、`role` |
| POST | `/api/invitations/{id}/accept` | 接受,返回 `{"workspace": …}` |
| POST | `/api/invitations/{id}/decline` | 拒绝,204 |

## 客户端

- `lib/views/invitations_controller.dart`:两端共用的控制器(收件箱状态 + 管理端调用)。
- `lib/ui/workspace_invitations.dart`:
  - `MemberInviteSection` —— 工作区设置对话框里的邀请表单,成功后提示「已发送邀请,对方接受后加入」,
    下方列出待接受的邀请(邮箱 + 「待接受」标记 + 撤回)。结果就地显示:壳层的错误横幅在模态遮罩后面看不到。
  - `ReceivedInvitationsCard` —— 侧栏工作区切换器下方,列出收到的邀请(工作区名、邀请人、接受/拒绝);
    接受后刷新工作区列表并切换过去。没有邀请时不占位置。
- 本地(离线)模式没有成员概念,两处都不出现。

## 先不做的

- **邮件通知**:没有邮件,受邀方要打开应用才看得到。真需要时在 `create_invitation` 之后调 mailer 即可。
- **过期时间 / 每工作区邀请上限**:当前邀请不过期,也不限量(不发邮件,所以无法拿它轰炸别人邮箱)。
- **CLI / MCP 的邀请管理**:mica-cli 当前没有成员管理命令,不受影响。
