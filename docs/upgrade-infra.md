# 基础设施手动升级(Postgres 大版本 / RustFS)

**这份文档管的是 `deploy/docker-compose.yml` 里那两个第三方镜像的版本号。**
应用自身的发版走 `docs/release.md`,和这里无关 —— 两件事**不要在同一次操作里做**,
出了问题分不清是谁的锅。

首次落地:**2026-08-28,PG 16.14 → 18.6-alpine,RustFS 1.0.0-beta.6 → 1.0.0-rc.3**。

---

## 先读这一段:为什么不能靠 `Deploy` workflow

平时上线是 `gh workflow run Deploy`,它把 tag 里的 compose 抄到节点再 `docker compose up -d`。
**对这两个镜像,那条路是错的:**

- **Postgres 的大版本号是数据目录的一部分。** 18 在 16 写的目录上会直接拒绝启动
  (`database files are incompatible with server`)然后 crash-loop。数据不会被吃掉,
  但服务就地躺下,而且**没有任何东西会自动修复它** —— 只能人来 dump/restore。
  补丁位(18.6 → 18.7)不受此限,那种升级走正常 deploy 即可。
- **RustFS 的磁盘格式这次没变**(下面有证据),所以它本可以随 deploy 滚;
  但它和 PG 的版本号写在同一个 compose 里,一起做才有意义。

所以顺序是**先按本文把数据搬好,再让 deploy 把新 compose 送上去**。

---

## 升级前必须知道的事实(实测/查证过的,不是推断)

### Postgres

| 事实 | 怎么确认的 | 为什么要紧 |
|---|---|---|
| 生产是 **16.14**,collation **`en_US.utf8`**,provider **libc(`c`)** | 节点上查 `SHOW server_version` / `pg_database` | restore 落地的库必须是同一套排序规则,否则索引顺序会变 |
| `postgres:18.6-alpine` 的 initdb 默认**同样是 `en_US.utf8` + libc** | 本地起容器实测 | 不需要传 `POSTGRES_INITDB_ARGS`,原样起就对齐 |
| 用到的扩展只有 `plpgsql` + `uuid-ossp`,**18.6-alpine 里都有** | 查 `pg_available_extensions` | 少一个扩展 restore 会中途失败 |
| 16.14 与 18.6 的 `pg_dump` **都会写 `\restrict` 指令**,两边 `psql` 也都认 | 本地实测 dump 文件 | 一份 dump **双向可还原**(升级与回滚共用);但**别喂给 16.10 / 17.6 之前的 psql**,那些版本不认这两行 |
| PG18 删掉了运行时参数 `lc_collate` | 实测 `SHOW lc_collate` 报错 | 脚本里读它的地方要改;我们没有 |
| **10019 篇文档的真实库 16.14 → 18.6 还原零错误**,三张表行数逐一相等,`_sqlx_migrations` 停在 26 | 本地用生产量级数据彩排 | 这条路走通过,不是纸上推演 |
| 应用在 18.6 上**全绿**:api-server 206、app-core 43+11+21、infra 32+1 | 同上,`DATABASE_URL` 指向 18.6 跑 | 证明的是**应用**兼容;上一行证明的是**数据**兼容,两者都要 |
| ⚠️ **PG18+ 官方镜像换了数据目录约定**:数据落在 `/var/lib/postgresql/<major>/docker`,**卷必须挂在 `/var/lib/postgresql` 这一层**;仍挂 `.../data` 会报 `Error: in 18+, these Docker images are configured to store database data in a format which is compatible with "pg_ctlcluster"` 然后 crash-loop | **升级当天在真机上撞到** | 本地彩排用的是不挂卷的 `docker run`,**覆盖不到这一条**;教训见文末 |
| 卷名带 compose 项目前缀:`mica_mica-prod-postgres`,不是 compose 文件 `volumes:` 段里的短名 | 同上 | 写错的话备份会建出一个空卷,而你以为备好了 |

### RustFS(beta.6 → rc.3,跨 9 个版本)

| 事实 | 出处 |
|---|---|
| **磁盘数据格式未变**:`XL_FILE_HEADER` / `XL_*_VERSION` / `BUCKET_METADATA_FORMAT` 五个常量两 tag 逐个比对相同 | 读 rustfs 两个 tag 的源码 |
| 官方明写"换镜像不改磁盘格式,启动不跑迁移" | `docs/operations/rolling-restart.md` @ 1.0.0-rc.3 |
| 我们用的 6 个环境变量(`RUSTFS_VOLUMES`/`ADDRESS`/`CONSOLE_ADDRESS`/`CONSOLE_ENABLE`/`ACCESS_KEY`/`SECRET_KEY`)**一个没改名** | 比对两 tag 的 config 常量表 |
| 两个文档化的不兼容点**都不适用于我们**:本地 SSE 的密钥信封格式(不用 KMS/SSE)、data-movement checksum 开关(默认关,且要 rebalance 才触发) | 同上 |
| beta.9 起 secret key 若仍是默认 `rustfsadmin` 则**启动即 abort** —— 我们的不是默认值,不触发 | beta.10 release note |
| 选**无后缀** tag(`1.0.0-rc.3`,alpine+musl,uid 10001),**不要 `-glibc`**(ubuntu+gnu) | 读两个 Dockerfile。和现在跑的 beta.6 同一条线,uid 一致所以卷属主不会出问题;换 libc 是与"升版本"无关的另一个变量,一次只动一个 |
| ⚠️ **回滚(rc.3 → beta.6)官方没有承诺**,也无 CI 覆盖;beta.6 → rc.3 这条路径官方 upgrade CI 同样没测过(只钉 rc.2 → current 一跳) | rustfs issue #6747 |
| ⚠️ rc.3 有一个已知"升级即起不来"回归(#6672),**只影响配过 bucket replication 的实例**,我们没有;修复在 rc.4 | rustfs issue #6672 |

> 两条 ⚠️ 合起来的意思:RustFS 这半边,**数据卷 tar 备份是唯一的回退保证**。别跳过。

---

## 操作步骤

全程在节点上(`ssh root@mica.cloudcele.com`,`cd /data/mica`)。
**容器名是 `mica-postgres-1`**(不是 `mica-postgres`,那是本地 dev 栈的)。

> ⚠️ **凡是会起容器的命令,一律用 `./dc`,不要用裸 `docker compose`。**
> 密码之类的机密在 `.env.secrets` 里,只有 `./dc` 会把它和 `.env` 一起传进去
> (`--env-file .env --env-file .env.secrets`)。
>
> `./dc` 本身只有三行(`exec docker compose --env-file .env --env-file .env.secrets "$@"`),
> 但它开头那段注释已经把这个坑写清楚了 —— **节点上就有,操作前先读它**。
>
> 2026-08-28 升级当天就栽在这:我用裸 `docker compose up -d postgres` 起了新库。
> compose 里写的是 `${POSTGRES_PASSWORD:-mica}`(**带默认值**,不是 `${VAR:?}`),
> 所以缺了 secrets 也不报错,而是**静默取默认值 `mica`,initdb 就用它把库建了出来**。
> 关键在于错得很"一致":api 的 `DATABASE_URL` 读的是同一个 `${POSTGRES_PASSWORD:-mica}`,
> 于是两边都用 `mica`,连得上、健康、跑了二十多分钟。直到下一次 deploy 用 `./dc`
> 把 `.env.secrets` 里的真密码传进来,两边才劈叉 —— `password authentication failed`,502,
> **而且回滚到上一版同样起不来**(库的密码是错的,与版本无关)。
>
> **这个坑的危险在于它不会当场报错**,而是把一颗雷埋到下一次部署。
> 补救不必重建库:`ALTER USER mica WITH PASSWORD ...` 对齐即可(见「回滚」下方)。

### 0. 预检(不改任何东西)

```bash
docker compose ps
docker exec mica-postgres-1 psql -U mica -d mica -tAc "SHOW server_version"
docker exec mica-postgres-1 psql -U mica -d mica -tAc \
  "SELECT datcollate, datctype, datlocprovider FROM pg_database WHERE datname='mica'"
docker exec mica-postgres-1 psql -U mica -d mica -tAc "SELECT extname FROM pg_extension"
docker exec mica-postgres-1 psql -U mica -d mica -tAc \
  "SELECT count(*) FROM document_yrs_base; SELECT count(*) FROM views;
   SELECT count(*) FROM users; SELECT max(version) FROM _sqlx_migrations"
df -h /data          # dump 约等于库大小,先确认盘装得下
```

对照上表核一遍,**行数记下来**(第 5 步要比对)。
**collation 或扩展与上表不符就停下** —— 那说明这台节点和文档记录的不是同一个东西。

### 1. 停止写入

```bash
docker compose stop api web mica-cli
docker compose ps        # 只剩 postgres / rustfs
```

api 停掉之后再 dump,否则备份与真实状态之间会差掉这期间的编辑。

### 2. 备份(两份,缺一不可)

```bash
TS=$(date +%Y%m%dT%H%M%S); echo "$TS"     # 记住它,后面几步都要用

# 2a. Postgres 逻辑备份 —— 用容器自带的 pg_dump(16),它的产物 18 和 16 都能吃
set -o pipefail          # 不加这行,pg_dump 失败也会留下一个"合法的空 gzip"
docker exec mica-postgres-1 pg_dump -U mica -d mica --no-owner --no-privileges \
  | gzip > /data/mica/pg16-pre18-$TS.sql.gz

# 校验:光 gzip -t 不够(截断的也能过),要看结构标记
zcat /data/mica/pg16-pre18-$TS.sql.gz | awk '
  /^COPY public\.document_yrs_base/   { t = 1 }
  /PostgreSQL database dump complete/ { d = 1 }
  END { exit !(t && d) }' && echo "dump OK" || echo "dump BAD — 停在这里"

# 2b. RustFS 数据卷 —— rc.3 回滚的唯一保证
docker compose stop rustfs
docker run --rm -v mica_mica-prod-rustfs:/data -v /data/mica:/backup alpine \
  tar czf /backup/rustfs-pre-rc3-$TS.tar.gz -C /data .
ls -lh /data/mica/*$TS*
```

`dump BAD` 就到此为止,不要继续。

### 3. 换掉 Postgres 的数据卷

新的大版本必须在**空目录**上 initdb。旧卷**复制一份留着**,不要直接删 ——
它是比 dump 更快的回滚路径。

```bash
docker compose down                       # 停全栈(卷不会被删)
# 卷名带 compose 项目前缀(`mica_`)。别照抄 compose 文件里 volumes: 段的短名 ——
# 那是项目内的名字,`docker volume` 认的是带前缀的全名,写错了下面这条 cp 会
# 建出一个空卷、而你以为备份好了。
docker volume ls --format '{{.Name}}' | grep mica-prod
docker run --rm -v mica_mica-prod-postgres:/from -v mica_mica-prod-postgres-pg16:/to alpine \
  sh -c 'cd /from && cp -a . /to'         # 整卷复制一份留底
docker volume rm mica_mica-prod-postgres       # 只删原卷;副本和 dump 都还在
```

> `docker volume rename` 不存在,所以用「复制一份 + 删原卷」达成同样效果。
> 复制会占掉与库等量的磁盘,先 `df -h` 看一眼。

### 4. 上新 compose(它带着 18.6 和 rc.3)

在**本地**触发部署,让它把新 compose 送上去:

```bash
gh workflow run Deploy --repo weironz/mica -f version=X.Y.Z
```

它会先跑一遍 `--check --diff` 彩排。**api 这时会起不来**(库是空的、还没 restore),
这是预期内的 —— 下一步就修。想更可控就手工 `docker compose up -d postgres` 只起库。

### 5. 还原数据

```bash
docker compose up -d postgres
until docker exec mica-postgres-1 pg_isready -U mica -d mica; do sleep 2; done
docker exec mica-postgres-1 psql -U mica -d mica -tAc "SHOW server_version"   # 应为 18.6

zcat /data/mica/pg16-pre18-$TS.sql.gz \
  | docker exec -i mica-postgres-1 psql -U mica -d mica -v ON_ERROR_STOP=1 -q
echo "restore exit=$?"       # 必须是 0
```

`ON_ERROR_STOP=1` 是必须的:少了它,restore 中途报错也会继续跑完,最后留下一个
**看起来成功的、缺东西的库**。

用步骤 0 记下的数字核对:

```bash
docker exec mica-postgres-1 psql -U mica -d mica -tAc \
  "SELECT count(*) FROM document_yrs_base; SELECT count(*) FROM views;
   SELECT count(*) FROM users; SELECT max(version) FROM _sqlx_migrations"
```

### 6. 起全栈并核验

```bash
docker compose up -d
curl -s https://mica.cloudcele.com/api/health          # 版本号 = 刚发的那版
docker compose logs --tail=50 api | grep -i "body-text search index warmed"
docker compose logs --tail=30 rustfs | tail -5
```

**冒烟测真正会坏的那几条路径**(版本号证明不了功能):

1. 登录 → 打开一篇带图的页面(presigned GET 通)。
2. 搜一个 2 字中文词(索引预热 + 搜索路由)。
3. 新建页面、粘一张图、刷新(presigned PUT + CreateBucket)。
4. 桌面端改一个设置、web 端刷新看是否跟随(v0.13.32 的设置同步)。

### 7. 收尾(观察期过后,别当天做)

```bash
docker volume rm mica_mica-prod-postgres-pg16
rm /data/mica/rustfs-pre-rc3-*.tar.gz            # dump 建议多留一阵
```

---

## 回滚

### Postgres

**最快**:把留底的卷换回去,并把镜像改回 `postgres:16-alpine`。

```bash
docker compose down
docker run --rm -v mica_mica-prod-postgres-pg16:/from -v mica_mica-prod-postgres:/to alpine \
  sh -c 'cd /from && cp -a . /to'
# 节点上的 docker-compose.yaml 改回 postgres:16-alpine。注意:下一次 deploy 会用
# tag 里的 compose 覆盖它,所以真要长期回退,仓库里的镜像号也得跟着退。
docker compose up -d
```

**代价**:旧卷停在步骤 3 那一刻,回滚会丢掉升级之后写入的一切。
所以第 6 步的冒烟要快 —— 别让用户在一个还没验收的库上工作半天。

### 密码对不上(裸 `docker compose` 起库的后遗症)

不用重建库,把角色密码对齐到 `.env.secrets` 就行。**别把密码回显到终端**:

```bash
cd /data/mica
PW=$(grep '^POSTGRES_PASSWORD=' .env.secrets | cut -d= -f2-)
printf 'ALTER USER mica WITH PASSWORD :%s;
' "'pw'" > /tmp/pw.sql
docker exec -i mica-postgres-1 psql -U mica -d mica -v pw="$PW" -f - < /tmp/pw.sql
rm -f /tmp/pw.sql
./dc up -d api web
```

`:'pw'` 是 psql 的变量引用,由 psql 负责转义 —— 直接把密码拼进 SQL 字符串,
遇到含引号的密码就会碎掉(或更糟,把 SQL 拼歪)。

### RustFS

```bash
docker compose stop rustfs
docker volume rm mica_mica-prod-rustfs && docker volume create mica_mica-prod-rustfs
docker run --rm -v mica_mica-prod-rustfs:/data -v /data/mica:/backup alpine \
  tar xzf /backup/rustfs-pre-rc3-$TS.tar.gz -C /data
# 镜像改回 rustfs/rustfs:1.0.0-beta.6,然后 docker compose up -d rustfs
```

---

## 下次做这件事时

- **补丁位升级不用走这份文档**(18.6 → 18.7、rc.3 → rc.4):改 compose 走正常 deploy 即可。
  只有**大版本**(PG 18 → 19)才需要整套 dump/restore。
- **先在本地彩排,而且要用与生产相同的挂载方式。** 本地 dev 栈(`docker-compose.yml`)用的是同一组镜像:把生产 dump
  拉下来在本地走一遍步骤 2→5,再让 `DATABASE_URL=...` 指着它跑 `cargo test`。
  2026-08-28 这次就是这么做的,当天问出了两件文档里查不到的事 ——
  18.6 的 initdb 默认 locale、以及 `pg_dump` 的 `\restrict` 指令。
  **但那次彩排用的是 `docker run` 不挂卷**,于是漏掉了 PG18 换挂载点这件事,
  真机第一次 `up -d` 直接 crash-loop。**彩排不复现挂载,就等于没覆盖挂载** ——
  下次照着 compose 起容器,别图省事。
- **一次只动一个变量。** 这次同时动 PG 和 RustFS,是因为两者都只写在这一个 compose 里、
  且 RustFS 侧已确认磁盘格式未变;如果 RustFS 的格式有变更,应当分两次做。
