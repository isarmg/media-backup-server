# Media Backup 运维文档

## 1. 生产拓扑与前置条件

正式 Server **只支持 Linux x86_64**，并要求 systemd、Python 3、GNU coreutils/tar，以及同机
Caddy/Nginx。`aarch64` 主机、非 Linux 主机和非 GNU Rust target 都会失败关闭，不存在交叉架构 fallback：

```text
Internet -> HTTPS reverse proxy -> 127.0.0.1:8080 Media Backup
                                      ├─ SQLite /var/lib/isarmg/media-backup/db/app.db
                                      └─ media  /var/lib/isarmg/media-backup/data
```

媒体保存为明文字节，必须启用主机/卷加密、最小权限和加密异地备份。

## 2. 构建与验证发行归档

维护者从干净的 `0.3.7` checkout 构建：

```bash
revision="$(git rev-parse HEAD)"
npm ci --prefix web
npm run build --prefix web
MEDIA_BACKUP_SOURCE_REVISION="$revision" cargo build --release --locked \
  -p media-backup-server --target x86_64-unknown-linux-gnu
mkdir -p "$PWD/dist"
./scripts/build-server-release.sh \
  "$PWD/target/x86_64-unknown-linux-gnu/release/media-backup-server" \
  "$revision" "$PWD/dist"
./scripts/test-deployment.sh "$PWD/dist/media-backup-server-0.3.7-x86_64-unknown-linux-gnu.tar.gz"
```

`web/dist` 是 Rust 编译输入，不是可复用的维护者缓存；从干净 checkout 构建时必须先用锁文件生成。完成只
表示归档通过身份和完整性检查，业务验收还必须由测试 Client 配对、上传一个测试文件、确认提交回执，并从
服务端读取或校验该资源。不要使用真实用户媒体作为发行 smoke。

Cargo release build script 拒绝其他 target；归档脚本还会核对构建主机为 Linux x86_64，并直接检查输入
二进制为 64 位 little-endian x86_64 ELF。构建器拒绝覆盖输出。归档 manifest 固定产品、版本、40 位
revision、target、`v2` 移动 API、`plain-v1`、Schema、移动 FFI、Web 与全树文件权限/大小/SHA-256；
额外文件、链接、特殊文件或硬链接别名均失败。

## 3. 安装

```bash
grep ' media-backup-server-0.3.7-x86_64-unknown-linux-gnu.tar.gz$' SHA256SUMS \
  | sha256sum --check -
tar -xzf media-backup-server-0.3.7-x86_64-unknown-linux-gnu.tar.gz
cd media-backup-server-0.3.7-x86_64-unknown-linux-gnu
./bin/media-backup-server release-identity
./bin/media-backup-server release-verify "$PWD"
sudo ./scripts/setup-wsl.sh
sudoedit /etc/isarmg/media-backup.env
sudo /opt/isarmg/media-backup/releases/0.3.7/scripts/start-server-wsl.sh
```

安装只允许创建缺失的 `/opt/isarmg/media-backup/releases/0.3.7`，不会覆盖或复用。同版本重装应先按
运维变更流程处理现有部署，而不是绕过 no-clobber。环境文件首次以 `0600` 排他创建；替换自动生成的
`BOOTSTRAP_ADMIN_USERNAME`、`BOOTSTRAP_ADMIN_PASSWORD`、`MEDIA_BACKUP_CREDENTIALS_KEY`、`METRICS_TOKEN` 并删除初始化标记后才能启动。登录候选 username
必须是 1–64 bytes 的可打印 ASCII；Foundation 会去除首尾 ASCII whitespace、转为 ASCII 小写，再要求
canonical 值为 3–64 bytes、首尾字母数字且全部字符仅为 `[a-z0-9._-]`，因此 `@`、Unicode、内部空白、
首尾分隔符都被拒绝。持久化和 Session 只接受已经 canonical 的值；`ADMIN_EMAIL` 不是配置别名。

## 4. 核心配置

| 变量 | 作用 | 生产要求 |
|---|---|---|
| `DATABASE_URL` | 当前 SQLite 路径 | 与媒体目录分离，路径父链不可是链接 |
| `DATA_DIR` | 原始媒体、缩略图和临时分块根 | 独立容量与 inode 监控 |
| `BIND` | HTTP 监听地址 | 推荐 `127.0.0.1:8080` |
| `BOOTSTRAP_ADMIN_USERNAME` | 无管理员时创建的初始管理员 username | 默认 admin；按 Foundation 规则规范化；已有管理员时不创建或覆盖身份 |
| `BOOTSTRAP_ADMIN_PASSWORD` | 初始管理员密码 | 仅无管理员时必填；已有管理员时不重置密码；生产由秘密管理器生成 |
| `MEDIA_BACKUP_CREDENTIALS_KEY` | 实例授权码信封加密主密钥 | 必填；Base64 解码后必须为 32 bytes，必须持久化并由秘密管理器保存 |
| `REQUIRE_HTTPS` | 强制可信 HTTPS 语义 | 必须为 `true` |
| `DEVELOPMENT` | 本机开发开关 | 生产必须为 `false` |
| `TRUSTED_PROXY_CIDRS` | 直接可信代理地址 | 仅列真实直连代理 |
| `METRICS_TOKEN` | `/metrics` 独立凭据 | 由秘密管理器生成和轮换 |

浏览器认证合同只有三条：`POST /api/v2/auth/login`、`GET /api/v2/auth/session`、
`POST /api/v2/auth/logout`。登录 body 精确为 `{username,password}`；登录和 session 成功体精确为
`{authenticated:true,user_id,username,role:"admin",csrf_token}`。备份账户的 `accounts.username` 只用于管理员识别存储租户，不再是客户端登录凭据；它与 `_sarmg_administrators.username` 是不同身份域。
权限。用户管理等业务位于 `/api/v2/admin/*`，移动端仍只使用 `/v2/*`。管理员 username 规范化、严格
当前 Argon2id、登录准入、Session/CSRF 生命周期、Cookie 和安全审计均由 Foundation 的
Admin Core、SQLite Store、Axum Adapter 拥有。空闲 30 分钟、绝对 12 小时、每管理员 32 个/全局 1024 个
Session 是固定平台策略，不提供产品级 TTL 配置。管理员登录来源使用真实 socket peer，不信任转发来源头。

Android/iOS 只向 `/v2/auth/bootstrap` 提交服务器地址、实例授权码和设备信息。授权码由管理员在备份账户下创建实例时生成，服务端保存密文和独立查找摘要；更换授权码会清除旧设备 Token 并要求重新配对。旧数据库和账户密码 bootstrap 不受支持，Server 遇到旧 Schema 会拒绝启动。

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
sudo /opt/isarmg/media-backup/releases/0.3.7/scripts/run-server-wsl.sh
```

启动脚本先检查 `uname`，二进制的 `serve-release` 再通过内核 `uname(2)` 检查 Linux x86_64，systemd 单元
还有 `ConditionArchitecture=x86-64`。三层任何一层不满足都必须在读取业务配置和创建状态前失败。

带环境配置运行：

```bash
media-backup-server doctor
```

`doctor` 检查当前元数据和 Schema 指纹、`integrity_check`、`foreign_key_check`、对象 Hash、上传恢复
状态、无引用 blob 回收意图、可回滚数据库写探针与可清理存储探针。若报告待回收 blob，先在服务停止
且同一环境配置下运行：

```bash
media-backup-server reconcile scan
media-backup-server doctor
```

`reconcile scan` 与服务启动/120 秒周期使用同一协调路径和运行锁；它会重试未完成 upload commit、无引用
blob rooted unlink/删行及 committed/orphan staging 清理，但不会周期性重新 Hash 全部已完成历史 blob。
完整内容校验属于显式 `doctor`；资源更新后，旧上传回执按不可变 `commit_blob_id` 验证，不要求资源当前
指针仍指向旧版本。永久删除响应 202 表示用户可见 metadata 已删除但
物理 blob 尚待该协调路径收口，不能盲目重放 DELETE；204 才表示本次已完成物理回收。指标只暴露聚合
数量和字节数，使用独立 Bearer Token。

## 6. 当前数据库合同

服务端软件版本是 `0.3.7`，但数据库合同独立保持不变：`product_metadata` 必须精确为
`application=media-backup`、`application_version=0.3.0`、
`schema_revision=3`，Schema SHA-256 为
`d65bf1183bc5bf3546738226c49711dbdbd520c5120a18df075273d5904bf51e`。移动队列对应
`media-backup-client` 与 SHA-256
`fb38736bbf8ac69eb694095e62302f73233e39df42cd2d38e3dd1284e2f02558`。

数据库只在主文件不存在时创建。已存在空文件、非当前元数据或结构漂移会在业务写入前拒绝，不能
现场手改指纹“修复”。

## 7. 当前状态备份与恢复

Media Backup 二进制不提供相关命令。当前 `sarmg-upgrade` 的 Media Backup 支持矩阵只覆盖
`0.2.0` / revision 1，**不支持**这里的 `0.3.0` / revision 3 数据库与配套 `DATA_DIR`，因此目前没有
受支持的产品级备份/恢复命令。不得用旧适配器、只复制 SQLite 或手改 identity 来绕过这一缺口；生产上线
前必须先为 `sarmg-upgrade` 增加并验证精确的 0.3.0/revision 3 状态适配器，使 SQLite 主文件、sidecar
与 `DATA_DIR` 作为同一一致性单元处理。适配器可用后仍应执行加密 3-2-1 备份和隔离恢复演练，恢复后先
运行离线验证与 `doctor` 再开放流量。

## 8. 移动端构建

移动端已移至 [media-backup-client](https://github.com/isarmg/media-backup-client)。
本 Server 仓库不构建移动包，不使用移动签名 Secrets。

## 9. 故障定位顺序

1. 检查固定发行树、manifest 和进程命令是否正确。
2. 检查 `/readyz`、Journal 和磁盘/inode 容量。
3. 检查代理真实 peer、TLS、`TRUSTED_PROXY_CIDRS` 和客户端时间。
4. 运行 `doctor`，区分数据库合同、文件系统、Hash 或上传恢复错误。
5. 移动端检查系统权限、后台任务限制、本地队列和安全凭据存储。
6. 若是版本/Schema 问题，停止服务并先核对 `sarmg-upgrade` 的精确支持矩阵；当前 0.3.0/revision 3
   不受支持，不能调用旧适配器，也不要把兼容代码加入 Server。

移动 Client 的当前已知边界：`retry_wait` 到期会重新准备源文件，不会复用仍持久化的 `prepared_json`；
若扫描器已按 `remove_source_after_prepare` 删除导出临时源，上传失败后可能持续报源不存在。准备失败或
`preparing` 崩溃也可能留下 partial part。采集 job ID、数据库行和对应目录证据后再处置；不得删除整个
`backup-staging-v0.2-r2/prepared/`，也不得把数据库中未经校验的 ID 直接拼为递归删除目标。

## 10. 安全事件

不要在公开 issue 中附带生产数据库、媒体、密码、Token 或日志中的私人路径。先隔离入口、保全只读
证据和摘要，再轮换管理员密码、设备 Token、API Key、指标 Token、TLS 私钥及可能泄露的主机凭据。
安全修复只面向当前版本。

## 11. 管理 Web 与 Foundation 门禁

管理 Web 必须使用 `.node-version` 指定的 Node `26.7.0`。Foundation 是构建期依赖；生产机不安装 npm
包，不访问 Foundation 仓库、registry 或 CDN。`build` 自带 `check:foundation` 前置门禁，因此正式顺序为：

```bash
npm ci --prefix web
npm run build --prefix web
MEDIA_BACKUP_SOURCE_REVISION="$(git rev-parse HEAD)" \
  cargo build --release --locked -p media-backup-server \
  --target x86_64-unknown-linux-gnu
```

门禁直接调用 Foundation `assertSarmgWebToolchain`，验证精确工具链及依赖/lockfile，并拒绝产品自有
登录外壳、存储凭据及私有字体/token 定义。管理页面使用共享 Shell/UI、认证客户端和 Maple 字体。
Vite 生成 HTML、JS、CSS、两个首屏 WOFF2、按需 CJK 分片与字体许可证；服务端的唯一内嵌资产清单同时用于
HTTP 响应和发行身份校验，发行包 `share/web/` 必须包含相同字节。字体经同源 `/admin/assets/` 路由
提供，类型为 `font/woff2`，不访问 CDN。必须先构建 Web，再构建 Server。

备份用户是设备上传业务账户，管理接口为 `/api/v2/admin/users`，不等同于平台管理员。产品页面不展示或
请求平台管理员列表；当前管理员只能从右上角人物图标进入 Foundation 账户设置。备份账户资料保存和密码重设是
两个独立操作，停用需显式确认；写请求失败不会自动重放，密码会清空，界面仅显示安全错误和 Request ID。

`npm run test:browser --prefix web` 对实际 dist 运行 Chromium/Firefox 验收，覆盖账户操作、
失败重试、业务账户与管理员账户入口隔离、无平台管理员面板、字体资产、键盘焦点及移动明暗主题 WCAG AA。首次运行先在
`web` 执行 `npx playwright install --with-deps chromium firefox`。

当前 Server Rust 固定 Foundation `=0.7.11` / `8f2a5c888bc5f543186ad58bc5b3cee4dc5b2602`；八个 Web 包使用
同版正式 Release tarball 与 lockfile integrity，不依赖相邻工作区。独立 CI 已通过，含 Server archive、
Android 编译及未签名 iOS 验证，见[消费者证据](https://github.com/isarmg/sarmg-foundation-server/blob/main/consumers/axum-0.7.0-evidence.md)。
这不代表这些主分支改动已重新发布为移动端或 Server 制品，也不代替原生签名和目标运行验收。
后续更新仍须复验锁图和发行身份；不得在线编辑 `share/web`、复制旧 dist、vendoring 共享 CSS 或加入兼容 fallback。
