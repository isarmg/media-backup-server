# xszs 运维文档

## 1. 生产拓扑与前置条件

正式 Server **只支持 Linux x86_64**，并要求 systemd、Python 3、GNU coreutils/tar，以及同机
Caddy/Nginx。`aarch64` 主机、非 Linux 主机和非 GNU Rust target 都会失败关闭，不存在交叉架构 fallback：

```text
Internet -> HTTPS reverse proxy -> 127.0.0.1:8080 xszs
                                      ├─ SQLite /var/lib/isarmg/xszs/db/app.db
                                      └─ media  /var/lib/isarmg/xszs/data
```

媒体保存为明文字节，必须启用主机/卷加密、最小权限和加密异地备份。

## 2. 构建与验证发行归档

维护者从干净的 `1.0.0` checkout 构建：

```bash
revision="$(git rev-parse HEAD)"
npm ci --prefix web
web/node_modules/.bin/xcss-build-server --config "$PWD/xcss-web-build.json" \
  --mode release --no-install --source-revision "$revision"
mkdir -p "$PWD/dist"
./scripts/build-server-release.sh \
  "$PWD/target/x86_64-unknown-linux-gnu/release/xszs" \
  "$revision" "$PWD/dist"
./scripts/test-deployment.sh "$PWD/dist/xszs-1.0.0-x86_64-unknown-linux-gnu.tar.gz"
```

`web/dist` 是 Rust 编译输入，不是可复用的维护者缓存；从干净 checkout 构建时必须先用锁文件生成。完成只
表示归档通过身份和完整性检查，业务验收还必须由测试 Client 配对、上传一个测试文件、确认提交回执，并从
服务端读取或校验该资源。不要使用真实用户媒体作为发行 smoke。

Cargo release build script 拒绝其他 target；归档脚本还会核对构建主机为 Linux x86_64，并直接检查输入
二进制为 64 位 little-endian x86_64 ELF。构建器拒绝覆盖输出。归档 manifest 固定产品、版本、40 位
revision、target、`v1` 移动 API、`plain-v1`、Schema、移动 FFI、Web 与全树文件权限/大小/SHA-256；
额外文件、链接、特殊文件或硬链接别名均失败。

## 3. 安装

```bash
grep ' xszs-1.0.0-x86_64-unknown-linux-gnu.tar.gz$' SHA256SUMS \
  | sha256sum --check -
tar -xzf xszs-1.0.0-x86_64-unknown-linux-gnu.tar.gz
cd xszs-1.0.0-x86_64-unknown-linux-gnu
./bin/xszs release-identity
./bin/xszs release-verify "$PWD"
sudo ./scripts/setup-wsl.sh
sudoedit /etc/isarmg/xszs.env
sudo systemd-run --wait --collect -p User=xszs -p Group=xszs -p EnvironmentFile=/etc/isarmg/xszs.env /opt/isarmg/xszs/releases/1.0.0/bin/xszs init
sudo /opt/isarmg/xszs/releases/1.0.0/scripts/start-server-wsl.sh
```

安装只允许创建缺失的 `/opt/isarmg/xszs/releases/1.0.0`，不会覆盖或复用。同版本重装应先按
运维变更流程处理现有部署，而不是绕过 no-clobber。环境文件首次以 `0600` 排他创建；替换自动生成的
`BOOTSTRAP_ADMIN_USERNAME`、`BOOTSTRAP_ADMIN_PASSWORD`、`XSZS_CREDENTIALS_KEY`、`METRICS_TOKEN` 并删除初始化标记后才能启动。登录候选 username
必须是 1–64 bytes 的可打印 ASCII；xcss 会去除首尾 ASCII whitespace、转为 ASCII 小写，再要求
canonical 值为 3–64 bytes、首尾字母数字且全部字符仅为 `[a-z0-9._-]`，因此 `@`、Unicode、内部空白、
首尾分隔符都被拒绝。持久化和 Session 只接受已经 canonical 的值；`ADMIN_EMAIL` 不是配置别名。

## 4. 核心配置

| 变量 | 作用 | 生产要求 |
|---|---|---|
| `DATABASE_URL` | 当前 SQLite 路径 | 与媒体目录分离，路径父链不可是链接 |
| `DATA_DIR` | 原始媒体、缩略图和临时分块根 | 独立容量与 inode 监控 |
| `BIND` | HTTP 监听地址 | 推荐 `127.0.0.1:8080` |
| `BOOTSTRAP_ADMIN_USERNAME` | 无管理员时创建的初始管理员 username | 默认 admin；按 xcss 规则规范化；已有管理员时不创建或覆盖身份 |
| `BOOTSTRAP_ADMIN_PASSWORD` | 初始管理员密码 | 仅无管理员时必填；已有管理员时不重置密码；生产由秘密管理器生成 |
| `XSZS_CREDENTIALS_KEY` | 实例授权码信封加密主密钥 | 必填；Base64 解码后必须为 32 bytes，必须持久化并由秘密管理器保存 |
| `REQUIRE_HTTPS` | 强制可信 HTTPS 语义 | 必须为 `true` |
| `DEVELOPMENT` | 本机开发开关 | 生产必须为 `false` |
| `TRUSTED_PROXY_CIDRS` | 直接可信代理地址 | 仅列真实直连代理 |
| `METRICS_TOKEN` | `/metrics` 独立凭据 | 由秘密管理器生成和轮换 |

浏览器认证合同只有三条：`POST /api/v1/auth/login`、`GET /api/v1/auth/session`、
`POST /api/v1/auth/logout`。登录 body 精确为 `{username,password}`；登录和 session 成功体精确为
`{authenticated:true,user_id,username,role:"admin",csrf_token}`。备份账户的 `accounts.username` 只用于管理员识别存储租户，不能作为客户端登录凭据；它与 `_common_administrators.username` 是不同身份域。
用户管理等业务位于 `/api/v1/admin/*`，移动端仍只使用 `/v1/*`。管理员 username 规范化、严格
当前 Argon2id、登录准入、Session/CSRF 生命周期、Cookie 和安全审计均由 xcss 的
Admin Core、SQLite Store、Axum Adapter 拥有。空闲 30 分钟、绝对 12 小时、每管理员 32 个/全局 1024 个
Session 是固定平台策略，不提供产品级 TTL 配置。管理员登录来源使用真实 socket peer，不信任转发来源头。

管理实例列表的 `GET /api/v1/admin/overview` 每页最多 50 个账户，响应额外包含
`previous_cursor`、`next_cursor` 和全局 `online_users`。其余全局 total/unlimited/used/pending/quota
由 SQL 聚合全部数据，不按当前页推断。按名称 NOCASE、原名、稳定 UUID 排序，使用原样返回的 cursor
正反翻页；名称重复或锚点行被删除仍可继续，刷新、创建和删除后返回第一页以核对最新列表。
`GET /api/v1/admin/users/{account_id}` 直接读取单个详情，`PUT` 保存后也不扫描全表。
当前 DDL 的唯一索引 `devices_account_unique_idx` 约束每个账户最多一个客户端实例，列表每页与详情嵌套
设备数量因此有界；不是在内存中截断其他设备。列表与详情响应预算为 1 MiB、时限 10 秒，错误/空页
与真正零实例分别表达，超出 JavaScript 精确整数范围的统计明确失败，不截断或重写存量数据。

管理 Web 的日志页使用 Server 本地时区的日历日期。`GET /api/v1/admin/logs` 返回
`{date,instance_id,logs,previous_cursor,next_cursor}`，省略 date 时默认 Server 当天；
`?date=YYYY-MM-DD` 指定日期，`&instance_id=UUID` 按实例筛选。每页最多 50 条，按审计序号倒序，
使用返回的 cursor 继续翻页；分页请求必须携带显式 date，游标与日期/实例不一致会被拒绝。
实例筛选覆盖设备 actor、属于设备的 API Key actor 及管理员 device entity，不按共同 account 猜测。
从实例详细信息进入日志会保留该实例上下文，“全部实例”恢复全局日志；刷新返回第一页。
日期边界由两个本地午夜分别换算为 UTC，夏令时日仍按完整日历日筛选；展示时间附 UTC 偏移。
浏览器单页响应预算 1 MiB、时限 10 秒，不积累历史页。账户级 `GET /v1/audit-events` 保持独立
账户授权及分页合同。结构化请求跨度和提交后实例事件提供稳定 instance_id，可独立分析。

Android/iOS 只向 `/v1/auth/bootstrap` 提交服务器地址、实例授权码和设备信息。授权码在管理员直接新建备份实例时生成，服务端保存密文和独立查找摘要；更换授权码会清除旧设备 Token 并要求重新配对。旧数据库和账户密码 bootstrap 不受支持，Server 遇到旧 Schema 会拒绝启动。

最小 Caddy 配置：

```caddyfile
media.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

防火墙必须阻止客户端绕过代理直连 Axum。代理应覆盖来源头；服务从真实 socket peer 开始由右向左
解析，未受信 peer 提供的转发头会被忽略。

## 5. 日常检查

```bash
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/readyz
sudo /opt/isarmg/xszs/releases/1.0.0/scripts/run-server-wsl.sh
```

启动脚本先检查 `uname`，二进制的 `run --release-root` 再通过内核 `uname(2)` 检查 Linux x86_64，systemd 单元
还有 `ConditionArchitecture=x86-64`。三层任何一层不满足都必须在读取业务配置和创建状态前失败。

带环境配置运行：

```bash
xszs doctor
```

`doctor` 检查当前元数据和 Schema 指纹、`integrity_check`、`foreign_key_check`、对象 Hash、上传恢复
状态、无引用 blob 回收意图、可回滚数据库写探针与可清理存储探针。若报告待回收 blob，先在服务停止
且同一环境配置下运行：

诊断最多保留 100,000 个目录条目或元数据项，索引与路径累计最多 64 MiB，目录深度最多 64。
对象 Hash 验证总读取量最多 1 TiB、总时限 600 秒，SQLite 查询使用三秒执行界限。
超过边界会完整报错，不返回被截断的成功结果，也不删除已有文件或数据库事实。

```bash
xszs reconcile scan
xszs doctor
```

`reconcile scan` 与服务启动/120 秒周期使用同一协调路径和运行锁；它会重试未完成 upload commit、无引用
blob rooted unlink/删行及 committed/orphan staging 清理，但不会周期性重新 Hash 全部已完成历史 blob。
完整内容校验属于显式 `doctor`；资源更新后，旧上传回执按不可变 `commit_blob_id` 验证，不要求资源当前
指针仍指向旧版本。永久删除响应 202 表示用户可见 metadata 已删除但
物理 blob 尚待该协调路径收口，不能盲目重放 DELETE；204 才表示本次已完成物理回收。指标只暴露聚合
数量和字节数，使用独立 Bearer Token。

## 6. 当前数据库合同

服务端软件版本是 `1.0.0`，但数据库合同独立保持不变：`product_metadata` 必须精确为
`application=xszs`、`application_version=1.0.0`、
`schema_revision=1`，Schema SHA-256 为
`0e37f8a3992b1904215d5f4c9ac428448752718506611f818482d890322d300d`。移动队列对应
`xszc` 1.0.0、schema revision 1 与 SHA-256
`87eb55ba9366cd06d5a2e0b69b5fd4c7a6eef59c381e9fd4ea340e9e04ef6dfb`。移动数据库身份由 Client
仓库定义，不属于 Server 启动时验证的数据库。

数据库仅由显式 `init` 在主文件不存在时创建；普通 `run` 不创建数据库。已存在空文件、非当前元数据或结构漂移会在业务写入前拒绝，不能
现场手改指纹“修复”。


## 8. 移动端构建

移动端已移至 [xszc](https://github.com/isarmg/xszc)。
本 Server 仓库不构建移动包，不使用移动签名 Secrets。

## 9. 故障定位顺序

1. 检查固定发行树、manifest 和进程命令是否正确。
2. 检查 `/readyz`、Journal 和磁盘/inode 容量。
3. 检查代理真实 peer、TLS、`TRUSTED_PROXY_CIDRS` 和客户端时间。
4. 运行 `doctor`，区分数据库合同、文件系统、Hash 或上传恢复错误。
5. 移动端检查系统权限、后台任务限制、本地队列和安全凭据存储。
6. 若是 Schema 问题，停止服务，保护原始数据，核对当前结构身份；不要手改 metadata。

移动 Client 的到期 `retry_wait` 会复用仍持久化的 `prepared_json` 和分块，不重新读取已删除的导出源。
没有准备结果的任务才重新读取源文件；准备在持久化前失败时可能留下未引用 generation，后续成功准备
会只回收同一 job 的旧 generation。采集 job ID、数据库行和对应目录证据后再处置；不得删除整个
`backup-staging-v1/`，也不得把数据库中未经校验的 ID 直接拼为递归删除目标。

## 10. 安全事件

不要在公开 issue 中附带生产数据库、媒体、密码、Token 或日志中的私人路径。先隔离入口、保全只读
证据和摘要，再轮换管理员密码、设备 Token、API Key、指标 Token、TLS 私钥及可能泄露的主机凭据。
安全修复只面向当前版本。

## 11. 管理 Web 与 xcss 门禁

管理 Web 必须使用 `.node-version` 指定的 Node `26.7.0`。xcss 是构建期依赖；生产机不安装 npm
包，不访问 xcss 仓库、registry 或 CDN。`build` 自带 `check:xcss` 前置门禁，因此正式顺序为：

```bash
npm ci --prefix web
web/node_modules/.bin/xcss-build-server --config "$PWD/xcss-web-build.json" --mode release --no-install
```

门禁直接调用 xcss `assertXcssWebToolchain`，验证精确工具链及依赖/lockfile，并拒绝产品自有
登录外壳、存储凭据及私有字体/token 定义。管理页面使用共享 Shell/UI、认证客户端和 Maple 字体。
Vite 生成 HTML、JS、CSS、两个首屏 WOFF2、按需 CJK 分片与字体许可证；服务端的唯一内嵌资产清单同时用于
HTTP 响应和发行身份校验。xcss 根据本次 dist 自动生成 inventory，并将快照内嵌到 binary；
发行包只带 `share/web-assets.json`，不重复携带 Web 原始字节。清单必须精确等于 executable 输出，
单独重写发行 manifest 不能授权其他清单。字体经同源 `/admin/assets/` 路由
提供，类型为 `font/woff2`，不访问 CDN。必须先构建 Web，再构建 Server。

管理端只呈现“备份实例”：点击新建后 `POST /api/v1/admin/instances` 使用默认名称，并在同一数据库事务中创建自动分配的
内部存储归属、100 GiB 默认配额、客户端实例和长期授权码；路径和配额可在详情页调整，不要求管理员先创建业务用户。
内部 `accounts` 仅作为上传数据的隔离与配额边界，不是登录身份，也不会在产品页面暴露账号或密码。当前管理员只能
从右上角人物图标进入 xcss 账户设置。实例创建后永久启用；写请求失败不会自动重放，界面仅显示安全错误和
Request ID。

管理 API 的单实例配额范围为 0～9007199254740991 bytes（0 表示不限）。概览容量合计也必须处于
JavaScript 安全整数范围；超出范围时返回结构化错误，不返回截断、环绕或不精确的计数。
管理页面接受小数 GiB，并四舍五入到最近的整数字节；大于零但不足半字节的输入会被拒绝，避免误设为“不限”。

“更换密码”先显示确认窗口，说明现有客户端凭据立即失效且需要重新配对；取消不会发送写请求。
确认后的请求执行期间禁止重复提交。若请求结果无法确认，界面提示关闭窗口并刷新实例信息，
核对当前密码后再决定下一步操作。

`npm run test:browser --prefix web` 对实际 dist 运行 Chromium/Firefox 验收，覆盖实例原子创建与配对、
失败重试、内部数据归属与管理员账户入口隔离、无平台管理员面板、字体资产、键盘焦点及移动明暗主题 WCAG AA。首次运行先在
`web` 执行 `npx playwright install --with-deps chromium firefox`。

当前 Server Rust 固定 xcss `=1.0.0` / `627d988a4ed471469ed4fdce8af0ea6b5c131ce6`；一个 @xcss/web 包使用
xcss 1.0.0 正式 Release tarball 与 lockfile integrity，不依赖相邻工作区。本仓库 CI 验证 Server、Web 与发行
归档；Android/iOS 构建和签名证据属于 Client 仓库，不能用 Server 构建结果代替。
后续更新仍须复验锁图和发行身份；不得在线编辑 `share/web-assets.json`、复制旧 dist、vendoring 共享 CSS 或加入兼容 fallback。

### Web 开发模式

```bash
npm ci --prefix web
web/node_modules/.bin/xcss-build-server --config "$PWD/xcss-web-build.json" --mode development --no-install
```

设置实验配置和 `DEVELOPMENT=true`、回环 `BIND` 后运行 `target/x86_64-unknown-linux-gnu/debug/xszs run`。
默认资源来自内嵌构建；设置 `XCSS_DEV_WEB_DIR="$PWD/web/dist"` 后可只重建 Web 热更新，或沿用
`npm run dev --prefix web` 的 Vite 代理。此覆盖仅在未绑定开发构建接受；正式 `run --release-root` 即使用于
HTTP 实验也拒绝外部 Web。资源 MIME、nosniff、HEAD、SHA-256 ETag 与缓存规则由 xcss 统一实现。

## 当前中立接口与旧版数据处理

当前版本只使用 `.state-instance.lock`、`.state-maintenance.lock`、`.state-maintenance-pending.json` 和 `.state-atomic-` 临时文件前缀；离线升级工具采用 `.release-upgrade` 工作目录。服务身份头为 `x-service`，健康状态中的公共源码修订字段为 `common_revision`。管理会话采用 `__Host-admin-xszs-session`，显式开发模式采用 `admin-xszs-session`；生产 Cookie 的 Secure、HttpOnly、SameSite、Path 和 CSRF 约束继续生效。资源清单格式为 `web-assets-v1`，公共数据库内部表及索引采用 `_common_` 前缀。

这些接口没有旧名称别名或旧版兼容分支。旧版升级前，先按本文的停服步骤停止服务、配套客户端及全部维护工具；确认全部进程退出后，完整备份配置、SQLite 数据库及其 WAL/SHM、业务文件和必要的私有凭据。备份包含敏感数据，应保留原有访问权限并离线保存。

保留旧数据目录，按当前安装步骤配置新的私有数据目录，执行显式 `init` 初始化，随后运行 `config validate`，再启动服务、登录管理页面并重新配对客户端。旧配置应人工审阅后填写当前字段，不能整体覆盖新目录。旧业务数据需要另行处理；当前版本不提供自动迁移。不得让旧、新版本同时写同一目录，不得通过删锁文件或修改数据库 metadata 强制启动；当前结构指纹包含实际表名、索引名和 SQL，仅改名称不能证明数据符合当前合同。
