# Media Backup Server 0.3.16 发行包使用手册

本文档只面向已经构建完成的 `media-backup-server-0.3.16-x86_64-unknown-linux-gnu.tar.gz`
正式发行包。它不是源码构建指南，也不描述 Android、iOS 或开发环境。本文出现的命令、路径和文件均以
当前 `0.3.16` 发行包的实际内容为准。

## 1. 先了解发行边界

- Server 只支持 **Linux x86_64（AMD64）**，目标为
  `x86_64-unknown-linux-gnu`。ARM、其他 CPU 架构、macOS、Windows 原生环境和非 GNU Linux 均不在
  Server 支持范围内。
- 正式运行方式是 systemd。服务固定使用非 root 账户 `isarmg-media`。
- 当前发行是不可变版本目录，不使用 `current` 软链接，也不会从任意路径动态选择二进制。
- 安装器只接受全新的目标。它不会覆盖、合并或修补已有 `0.3.16` 发行目录，也不会覆盖已有 systemd
  unit。
- 本轮仅支持唯一当前版本，不提供旧版本升级或兼容路径。当前状态的离线校验、备份和恢复由独立的
  `sarmg-upgrade` 项目负责，不能通过修改数据库版本号、发行 manifest 或软链接绕过检查。
- 媒体对象以明文字节写入数据目录。生产主机必须另行提供磁盘或卷加密、最小权限和加密异地备份。

## 2. 主机前置条件

安装前确认主机满足以下条件：

| 条件 | 当前要求 | 不满足时的结果 |
|---|---|---|
| 操作系统与架构 | Linux x86_64 | 安装脚本和 Server 均会失败关闭 |
| 初始化系统 | systemd | 无法安装和启动正式服务 |
| 管理权限 | 可使用 `sudo` 成为 root | 无法创建系统账户、配置、状态目录和 unit |
| 基础命令 | Bash、Python 3、GNU coreutils、GNU tar、gzip、`getent`、`useradd`、`groupadd`、`curl` | 安装、校验或健康检查无法完成 |
| 入口 TLS | 同机或可信直连的 Caddy/Nginx 等反向代理 | 生产安全模式下管理页和 API 请求会被拒绝 |
| 存储 | 为数据库和媒体数据分别预留容量、inode 和备份空间 | 上传、缩略图或 SQLite 写入可能失败 |

建议在独立服务主机上部署。不要把不可信用户赋予 `/opt/isarmg`、`/etc/isarmg`、
`/var/lib/isarmg` 或 systemd unit 的写权限。

## 3. 发行包内容

解压后只有一个顶层目录：

```text
media-backup-server-0.3.16-x86_64-unknown-linux-gnu/
├── bin/media-backup-server
├── config/media-backup.env.example
├── docs/feature-inventory-and-tradeoffs.md
├── scripts/
│   ├── setup-wsl.sh
│   ├── start-server-wsl.sh
│   ├── run-server-wsl.sh
│   └── verify-server-wsl.sh
├── share/web/
│   ├── index.html
│   └── assets/
│       ├── admin.css
│       ├── admin.js
│       ├── MapleMono.woff2
│       ├── MapleMono-Italic.woff2
│       └── MapleMono-OFL.txt
├── systemd/media-backup.service
├── LICENSE
├── README.md
└── release-manifest.json
```

其中：

- `release-manifest.json` 绑定产品、版本、源码 revision、目标架构、Schema、移动 FFI、管理 Web 和每个
  文件的相对路径、权限、大小、SHA-256。
- `bin/media-backup-server` 同时实现独立发行校验和服务启动时校验。仅替换 manifest 或仅替换二进制都
  无法组成有效发行。
- `share/web/` 是 Server 编译时绑定的当前 React/Vite 管理 Web。不得在线编辑或替换。
- `docs/feature-inventory-and-tradeoffs.md` 是当前完整功能与取舍清单，说明每项能力的分类、复杂度、
  删除后果和验证要求。

发行树不允许出现缺失文件、额外文件、符号链接、硬链接别名、特殊文件、权限漂移或内容漂移。任何这类
变化都会使校验或启动失败。需要改配置时只编辑 `/etc/isarmg/media-backup.env`，不要编辑发行树。

## 4. 下载后先校验

先从可信发布渠道取得归档的 SHA-256，再比较本地文件。不要把归档旁边由未知来源提供的摘要当作可信
根。

```bash
archive='media-backup-server-0.3.16-x86_64-unknown-linux-gnu.tar.gz'
expected_sha256='从可信发布页复制的64位小写SHA-256'
printf '%s  %s\n' "$expected_sha256" "$archive" | sha256sum --check --strict -
```

检查归档只包含预期顶层目录，然后解压：

```bash
tar -tzf "$archive"
tar -xzf "$archive"
cd media-backup-server-0.3.16-x86_64-unknown-linux-gnu
```

在安装前读取二进制身份并验证整个解压目录：

```bash
./bin/media-backup-server release-identity
./bin/media-backup-server release-verify "$PWD"
```

`release-identity` 应报告以下关键字段：

- `product`：`media-backup-server`
- `version`：`0.3.16`
- `target`：`x86_64-unknown-linux-gnu`
- `api_version`：`v2`
- `storage_encoding`：`plain-v1`
- `server_schema_revision`：`5`

`source_revision` 必须是 40 位小写十六进制 Git revision。`release-verify` 成功时只输出一行以
`MEDIA_BACKUP_RELEASE_VERIFIED_V1` 开头的身份；失败时不要继续安装，也不要手工改 manifest。

## 5. 全新安装

在发行包顶层运行：

```bash
sudo ./scripts/setup-wsl.sh
```

脚本执行顺序如下：

1. 检查 Linux x86_64、root 权限、发行包物理路径和完整 payload。
2. 在任何安装写入前拒绝链接路径、特殊文件、非空发行目标、已有版本目录和已有 systemd unit。
3. 创建或核对专用组与账户 `isarmg-media`；账户 home 固定为
   `/var/lib/isarmg/media-backup`，登录 shell 必须是 `nologin`。
4. 把经过校验的完整发行复制到 `/opt/isarmg/media-backup/releases/0.3.16`，设为 root 所有，并再次
   运行 installed-release 校验。
5. 创建 `/var/lib/isarmg/media-backup/db` 与 `/var/lib/isarmg/media-backup/data`，权限为 `0750`，
   所有者为 `isarmg-media:isarmg-media`。
6. 若配置不存在，以 root、`0600`、单硬链接方式排他创建 `/etc/isarmg/media-backup.env`，生成彼此
   独立的 256-bit 初始管理员密码和指标 Token；脚本不会把秘密打印到终端。
7. 排他安装 `/etc/systemd/system/media-backup.service`，执行 `systemctl daemon-reload`，但不会启动
   服务。

正常完成时固定布局为：

```text
/opt/isarmg/media-backup/releases/0.3.16/   # root 拥有的不可变发行
/etc/isarmg/media-backup.env              # root:root，0600，唯一可编辑配置
/etc/systemd/system/media-backup.service  # root:root，0644，发行包中的 unit
/var/lib/isarmg/media-backup/db/           # SQLite 状态
/var/lib/isarmg/media-backup/data/         # 媒体、缩略图和上传暂存状态
/run/isarmg/media-backup/                  # systemd 创建的运行时目录
```

安装是 one-shot/no-clobber。再次执行不会覆盖同版本，也不会用“内容相同”作为复用依据。若目标已有其他
内容，先查清来源；不要删除或改名后强行安装。已有配置文件只有在它是安全的普通单链接文件时才可能被
保留，但已有 unit 或非空应用目录会使安装拒绝。

## 6. 首次启动前配置

安装器创建的配置包含一行：

```text
# INITIAL-SECRETS-MUST-BE-REPLACED
```

在首次启动前必须用 `sudoedit` 安全记录或替换三个初始秘密，并删除这行 marker：

```bash
sudoedit /etc/isarmg/media-backup.env
```

不要把配置复制到工单、聊天、Shell history 或公开仓库。配置文件必须保持 root 所有、权限 `0600`、
单硬链接且不是符号链接；启动脚本会再次检查这些条件。

### 6.1 配置项

| 变量 | 默认示例或语义 | 生产约束 |
|---|---|---|
| `DATABASE_URL` | `sqlite:///var/lib/isarmg/media-backup/db/app.db` | 必须指向当前 SQLite；父目录链不得用链接代换 |
| `DATA_DIR` | `/var/lib/isarmg/media-backup/data` | 必须与发行树分离；持续监控容量和 inode |
| `BIND` | `127.0.0.1:8080` | 推荐仅监听 loopback，由反向代理对外提供 TLS |
| `BOOTSTRAP_ADMIN_USERNAME` | `admin` | 仅在没有管理员时创建初始身份；已有身份不被覆盖 |
| `BOOTSTRAP_ADMIN_PASSWORD` | 安装时随机生成 | 仅初始化时必填；已有管理员时不会重置其密码 |
| `MEDIA_BACKUP_CREDENTIALS_KEY` | 32 bytes 随机值的 Base64 | 必填且必须持久化；用于实例授权码信封加密，不得与其他 Token/密码复用 |
| `MAX_PART_BYTES` | `67108864` | 单上传分块上限；同时影响请求 body 上限和内存/并发压力 |
| `UPLOAD_GLOBAL_CONCURRENCY` | 未配置时 `16` | 正整数；限制全局并行上传处理 |
| `UPLOAD_PER_ACCOUNT_CONCURRENCY` | 未配置时 `4` | 正整数且不得大于全局值 |
| `REQUIRE_HTTPS` | `true` | 生产必须为 `true` |
| `DEVELOPMENT` | `false` | 生产必须为 `false`；为 `true` 时只允许 loopback bind |
| `TRUSTED_PROXY_CIDRS` | `127.0.0.1/32,::1/128` | 只填写会直接连接 Server 的真实可信代理地址或网段 |
| `METRICS_TOKEN` | 安装时随机生成 | 独立 Bearer Token；空值会关闭 `/metrics` |
| `RUST_LOG` | `media_backup_server=info,tower_http=info` | 控制日志级别；不要开启会泄露敏感数据的临时调试日志 |

### 6.2 管理员身份合同

Server 与管理 Web 只有一个角色：`admin`。不存在访客、普通后台角色或邮箱登录。

- 登录请求精确为 `{username,password}`。
- 登录候选 username 长度为 1–64 bytes，必须是可打印 ASCII；规范化会去除首尾 ASCII 空白并转为
  ASCII 小写。
- 规范化后的 canonical username 长度为 3–64 bytes，首尾必须是字母或数字，中间字符只允许
  `[a-z0-9._-]`。
- `@`、Unicode、内部空白、控制字符和首尾分隔符均被拒绝。
- 管理员密码必须为 12–1024 bytes，且不得包含 ASCII 控制字符。
- 管理员浏览器 Session 与移动备份账户、设备 Bearer Token、API Key 是相互隔离的身份域；不得把
  `BOOTSTRAP_ADMIN_USERNAME` 当作移动账户配置，也不得复制凭据实现“兼容”。

## 7. 配置 HTTPS 反向代理

生产流量应为：

```text
浏览器或移动客户端
        │ HTTPS
        ▼
可信反向代理
        │ HTTP，仅同机 loopback 或受控私网
        ▼
127.0.0.1:8080 Media Backup
```

最小 Caddy 示例：

```caddyfile
media.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

代理必须覆盖而不是盲目信任客户端传入的转发头，并正确传递 HTTPS 语义。Server 只在 socket peer
落入 `TRUSTED_PROXY_CIDRS` 时信任对应转发信息。把互联网网段或任意客户端地址加入该列表会使攻击者
伪造来源或安全协议；不要为了消除 4xx 而放宽为全网段。

防火墙必须阻止外部客户端绕过代理直接访问 Server。TLS 证书、私钥、HSTS 和公网访问控制由反向代理
负责，但 Server 仍会在业务入口强制验证安全传输语义。

## 8. 启动与启用服务

完成秘密替换、删除 marker 并配置反向代理后运行：

```bash
sudo /opt/isarmg/media-backup/releases/0.3.16/scripts/start-server-wsl.sh
```

该脚本会：

1. 核对主机架构、固定发行目录的 root 所有权和不可写属性；
2. 由发行内真实二进制复核 manifest、身份和 payload；
3. 核对已安装 unit 与发行内 unit 字节一致；
4. 核对配置为 root 所有、`0600`、单硬链接且 marker 已删除；
5. 执行 `systemctl enable --now media-backup.service` 并输出完整状态。

systemd 最终只执行固定命令：

```text
/opt/isarmg/media-backup/releases/0.3.16/bin/media-backup-server serve-release /opt/isarmg/media-backup/releases/0.3.16
```

服务进程会再次验证它确实物理位于该发行根内，然后才读取业务配置和创建运行状态。直接复制二进制到
其他位置、使用普通 `serve` 或通过软链接启动均会被拒绝。

若只想启动已有服务并持续查看 Journal，可运行：

```bash
sudo /opt/isarmg/media-backup/releases/0.3.16/scripts/run-server-wsl.sh
```

此脚本不会替代首次安装或首次启用；它同样先做发行、unit 和配置检查，再调用 `systemctl start`，最后
以前台方式跟随该 unit 的日志。使用 `Ctrl+C` 只结束日志跟随，不等于停止服务。

## 9. 健康检查与验收

本机基础检查：

```bash
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/readyz
```

- `/healthz` 仅以状态码表示存活，不返回内部信息。
- `/readyz` 同时探测 SQLite 与数据目录；返回 `503` 时不能接入业务流量。

使用包内验收脚本检查健康端点与管理页：

```bash
/opt/isarmg/media-backup/releases/0.3.16/scripts/verify-server-wsl.sh
```

默认检查 `http://127.0.0.1:8080`，并用 `X-Forwarded-Proto: https` 模拟可信代理语义。若监听地址或
代理拓扑不同，可只为本次命令覆盖：

```bash
MEDIA_BACKUP_VERIFY_URL='http://127.0.0.1:18080' \
MEDIA_BACKUP_VERIFY_FORWARDED_PROTO='https' \
  /opt/isarmg/media-backup/releases/0.3.16/scripts/verify-server-wsl.sh
```

预期输出为 `health=200 admin_page=200`。该检查不能替代从真实公网域名验证证书链、DNS、代理、Cookie
和登录流程。

管理 Web 入口是 `/admin`。管理员认证 API 是：

- `POST /api/v2/auth/login`
- `GET /api/v2/auth/session`
- `POST /api/v2/auth/logout`

移动端 API 位于 `/v2/*`。备份账户只是媒体归属租户；每个客户端实例使用管理员生成的独立授权码配对，
成功后获得设备 Bearer Token。更换授权码会撤销旧 Token 并要求重新配对，不接受旧的账户密码 bootstrap。

## 10. 日常运维

### 10.1 systemd 与日志

```bash
sudo systemctl status media-backup.service --no-pager --full
sudo journalctl --unit media-backup.service --since today
sudo journalctl --unit media-backup.service --follow
```

不要把完整配置、密码、Bearer Token、Cookie、私人媒体路径或数据库内容粘贴到公开 issue。收集日志时
先做脱敏，并保留时间范围、HTTP 状态、请求 ID、磁盘状态和只读摘要。

### 10.2 指标

配置非空 `METRICS_TOKEN` 后，`/metrics` 需要独立 Bearer Token：

```bash
curl --fail \
  --header 'Authorization: Bearer <独立的METRICS_TOKEN>' \
  http://127.0.0.1:8080/metrics
```

指标只用于监控，不要复用管理员密码、浏览器 Cookie、移动设备 Token 或 API Key。若不需要指标，把
配置值留空即可关闭该端点；修改配置后通过受控维护窗口重启服务。

### 10.3 容量与状态

持续监控：

- `/var/lib/isarmg/media-backup/db` 所在文件系统的容量、inode、I/O 延迟和 SQLite sidecar；
- `/var/lib/isarmg/media-backup/data` 的媒体字节、缩略图、分块暂存与孤儿回收积压；
- `/readyz`、进程重启次数、上传错误率、活动上传数和存储字节数；
- 反向代理证书到期时间、5xx、请求 body 限制和 upstream timeout。

数据库和媒体目录构成同一个逻辑一致性单元。不得只复制 `app.db` 而忽略 SQLite sidecar，也不得只
复制媒体目录后假设 metadata 会自动重建。

### 10.4 离线诊断与协调

`doctor` 会检查当前产品元数据、精确 Schema、SQLite integrity/foreign keys、对象 Hash、上传恢复
状态、无引用 blob、数据库可回滚写探针和可清理存储探针。`reconcile scan` 会处理未完成 upload
commit、待回收 blob 和孤儿 commit staging。

它们读取与服务相同的环境配置和状态路径。只应在明确的维护窗口内、使用受控运维方式运行，避免与正在
处理业务流量的进程争用；运行前先保存证据并确认当前发行身份。不要把未经验证的数据库 ID 拼进递归文件
删除命令，也不要手工清空整个暂存目录。

## 11. 备份、恢复与版本变更

Media Backup Server 发行包不执行备份、恢复、Schema 迁移、跨版本转换或回滚。正式流程是：

1. 记录当前发行 identity、服务状态和存储容量；
2. 阻断新流量并停止服务；
3. 由独立 `sarmg-upgrade` 仓库的当前显式工作流同时处理 SQLite 主文件及 sidecar、媒体数据和配置；
4. 在隔离环境校验摘要、结构和恢复可用性；
5. 只把目标当前版本认可的完整状态交给目标发行；
6. 运行离线验证与 readiness 检查，再恢复流量。

每个产品版本都是全新合同。不要在 Server 中加入旧字段、旧路径、旧 Schema、旧用户名格式、旧发行
别名或双读 fallback，也不要通过手工编辑 `product_metadata` 冒充当前数据库。

建议执行 3-2-1 备份策略，备份在传输和静态状态下都加密，并定期做完整恢复演练。只“成功生成归档”
不能证明可恢复。

## 12. 常见故障定位

### 12.1 安装器报告发行无效

可能原因包括下载损坏、错误架构、解压后文件被改动、权限改变、额外文件、链接或特殊文件。重新从可信
渠道下载并核对外部 SHA-256；不要修补 manifest 让坏包通过。

### 12.2 安装器拒绝目标已存在

这是 no-clobber 保护，不是可忽略警告。保存现场并确认该目录或 unit 的来源。本包没有覆盖安装模式，
也不提供旧版本升级；不要删除既有数据或修改 manifest 绕过保护。

### 12.3 启动脚本要求替换初始秘密

编辑 `/etc/isarmg/media-backup.env`，妥善保存安装器生成的 `BOOTSTRAP_ADMIN_PASSWORD`、
`MEDIA_BACKUP_CREDENTIALS_KEY` 和 `METRICS_TOKEN`，确认三者属于不同用途后删除精确 marker 行。保持文件 root 所有、
`0600`、普通文件且只有一个硬链接。后续不得丢失或随意更换授权码主密钥，否则已保存的实例授权码无法解密。

### 12.4 服务启动后立即退出

按以下顺序检查：

1. `systemctl status` 与 Journal 的第一条错误；
2. 主机是否为 Linux x86_64；
3. 固定发行目录、unit 和配置的所有权/权限是否漂移；
4. `DATABASE_URL`、`DATA_DIR` 和 `BIND` 是否有效；Session 生命周期由 Foundation 固定；
5. 数据库是否是精确的当前产品、版本、Schema 和对象指纹；
6. 数据目录是否可由 `isarmg-media` 访问，以及容量/inode 是否耗尽。

遇到非当前格式或 Schema 漂移必须停止并保留现场。本轮没有旧格式转换流程；不要现场执行
`ALTER TABLE`、修改版本元数据或添加兼容列。当前状态损坏只能从经验证的当前格式备份恢复。

### 12.5 `/healthz` 正常但 `/readyz` 为 503

进程活着，但 SQLite 查询或数据目录探针失败。检查磁盘、挂载、权限、只读状态、文件系统错误和 Journal。
在 readiness 恢复前不要让代理继续发送业务流量。

### 12.6 管理页或 API 返回安全传输错误

确认请求确实经过 TLS，Server 的 socket peer 是 `TRUSTED_PROXY_CIDRS` 中的直接代理，代理覆盖并传递
正确协议。不要把 `REQUIRE_HTTPS` 改为 `false`，也不要把 `DEVELOPMENT` 用于非 loopback 生产监听。

### 12.7 登录失败

确认使用 username 而不是邮箱；检查 canonical 字符规则、密码长度、浏览器 Cookie、代理 HTTPS 语义和
服务器时间。不要创建第二角色、邮箱别名或旧登录 fallback。

### 12.8 上传或存储失败

检查数据目录容量/inode、分块上限、全局/账户并发限制、反向代理 body 限制和 timeout。永久删除返回
`202` 表示用户可见 metadata 已删除但物理 blob 仍待协调路径收口；不要盲目重放删除或手工删除文件。

## 13. 安全事件最小处置

1. 在代理层隔离受影响入口，避免继续扩大写入或泄漏；
2. 保全只读日志、发行 identity、文件摘要、时间线和数据库/媒体一致性证据；
3. 根据影响范围轮换管理员密码、指标 Token、设备 Token、API Key、TLS 私钥及主机凭据；
4. 使用独立的当前状态恢复流程从经过验证的一致性备份恢复；
5. 只为当前版本修复，不在产品代码中加入旧版本兼容路径。

不要公开上传生产数据库、媒体、配置或秘密。不要在尚未保全证据时批量删除日志、数据库 sidecar 或暂存
目录。

## 14. 验收清单

交付前逐项确认：

- [ ] 主机是 Linux x86_64，systemd 与所需基础命令可用。
- [ ] 归档外部 SHA-256 来自可信渠道并已核对。
- [ ] `release-identity` 与 `release-verify` 成功，产品、版本、target 和 source revision 正确。
- [ ] 安装目标此前为空，没有通过覆盖、软链接或手工复制规避 no-clobber。
- [ ] 发行位于固定 `/opt/isarmg/media-backup/releases/0.3.16`，未被在线编辑。
- [ ] 配置位于 `/etc/isarmg/media-backup.env`，root 所有、`0600`、单硬链接。
- [ ] 三个初始秘密已按不同用途安全保存或替换，初始化 marker 已删除。
- [ ] Server 只监听受控地址，公网只能通过可信 TLS 反向代理访问。
- [ ] `TRUSTED_PROXY_CIDRS` 只包含真实直连代理。
- [ ] systemd 服务以 `isarmg-media` 运行，unit 与发行内容一致。
- [ ] `/healthz`、`/readyz` 和包内验收脚本均通过；不存在旧健康路径别名。
- [ ] 从真实域名验证证书链、管理页、username 登录、Session 和退出。
- [ ] 数据库与媒体容量、inode、日志、指标和证书到期监控已配置。
- [ ] 一致性备份、隔离恢复演练和 `sarmg-upgrade` 责任人已明确。

以上清单全部通过，才表示当前 `0.3.16` Server 发行具备上线条件。
