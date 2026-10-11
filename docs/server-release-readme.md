# 安装 xszs 服务端

本手册随 `xszs-1.0.1-x86_64-unknown-linux-gnu.tar.gz` 打包，使用包内程序和脚本完成全新安装。
目标是通过 HTTPS 登录管理页，并让 xszc 手机客户端完成一份测试媒体的上传和读取。

## 主机准备

- Linux AMD64 GNU（`x86_64-unknown-linux-gnu`），systemd，sudo 管理权限。
- Bash、Python 3.11+、GNU coreutils/tar、gzip、getent、useradd、groupadd 和 curl。
- HTTPS 域名及同机或可信直连的 Caddy/nginx。
- 数据库和媒体存储的容量、inode 及访问权限。

媒体以明文字节保存，主机或存储卷应提供加密并限制读取权限。安装器只接受全新发行目录与 unit；已有部署先保留现场，按其实际状态处理。

## 1. 校验下载

把归档与同版 `SHA256SUMS` 放在同一目录：

```sh
grep ' xszs-1.0.1-x86_64-unknown-linux-gnu.tar.gz$' SHA256SUMS \
  | sha256sum --check -
tar -tzf xszs-1.0.1-x86_64-unknown-linux-gnu.tar.gz
tar -xzf xszs-1.0.1-x86_64-unknown-linux-gnu.tar.gz
cd xszs-1.0.1-x86_64-unknown-linux-gnu
./bin/xszs release-identity
./bin/xszs release-verify "$PWD"
```

摘要应来自可信发布渠道。校验成功后，身份应包含 `product=xszs`、`version=1.0.1`、
`target=x86_64-unknown-linux-gnu`、`api_version=v1`、`storage_encoding=plain-v1`、结构修订 1 和完整源码 SHA。
完整校验输出以 `XSZS_RELEASE_VERIFIED_V1` 开头。校验失败时重新核对归档来源与完整性。

## 2. 安装

在刚解压的包根目录执行：

```sh
sudo ./scripts/setup-wsl.sh
sudoedit /etc/isarmg/xszs.env
```

安装器创建专用 `xszs` 用户、受保护的配置和状态目录，安装 unit 后暂不启动。预期布局：

```text
/opt/isarmg/xszs/releases/1.0.1/   只读发行树
/opt/isarmg/xszs/current          指向同版发行树的受控链接
/etc/isarmg/xszs.env             root:root 0600
/etc/systemd/system/xszs.service root:root 0644
/var/lib/isarmg/xszs/db/          数据库，xszs:xszs 0700
/var/lib/isarmg/xszs/data/        媒体与暂存，xszs:xszs 0700
/run/isarmg/xszs/                systemd 运行时目录
```

配置在发行树之外修改；发行树、manifest 和内嵌 Web 清单保持原样，启动时会重新核验。

## 3. 审阅配置

在 `/etc/isarmg/xszs.env` 中确认或替换独立的初始管理员密码、`XSZS_CREDENTIALS_KEY` 与 `METRICS_TOKEN`。
将它们保存在受控秘密存储中，然后删除已审阅的 `# INITIAL-SECRETS-MUST-BE-REPLACED` 行。
授权码密文依赖 `XSZS_CREDENTIALS_KEY`；该值必须与当前数据持续匹配。
配置保持 root 所有、`0600`、单硬链接普通文件。

### 配置项

| 变量 | 默认示例或语义 | 生产约束 |
|---|---|---|
| `DATABASE_URL` | `sqlite:///var/lib/isarmg/xszs/db/app.db` | 必须指向当前 SQLite；父目录链不得用链接代换 |
| `DATA_DIR` | `/var/lib/isarmg/xszs/data` | 必须与发行树分离；持续监控容量和 inode |
| `BIND` | `127.0.0.1:8080` | 推荐仅监听 loopback，由反向代理对外提供 TLS |
| `BOOTSTRAP_ADMIN_USERNAME` | `admin` | 仅由显式 init 创建初始身份；已有身份不被覆盖 |
| `BOOTSTRAP_ADMIN_PASSWORD` | 安装时随机生成 | 仅初始化时必填；已有管理员时不会重置其密码 |
| `XSZS_CREDENTIALS_KEY` | 32 bytes 随机值的 Base64 | 必填且必须持久化；用于实例授权码信封加密，不得与其他 Token/密码复用 |
| `MAX_PART_BYTES` | `67108864` | 单上传分块上限；同时影响请求 body 上限和内存/并发压力 |
| `UPLOAD_GLOBAL_CONCURRENCY` | 未配置时 `16` | 正整数；限制全局并行上传处理 |
| `UPLOAD_PER_ACCOUNT_CONCURRENCY` | 未配置时 `4` | 正整数且不得大于全局值 |
| `REQUIRE_HTTPS` | `true` | 生产必须为 `true` |
| `DEVELOPMENT` | `false` | 生产必须为 `false`；为 `true` 时只允许 loopback bind |
| `TRUSTED_PROXY_CIDRS` | `127.0.0.1/32,::1/128` | 只填写会直接连接 Server 的真实可信代理地址或网段 |
| `METRICS_TOKEN` | 安装时随机生成 | 独立 Bearer Token；空值会关闭 `/metrics` |
| `RUST_LOG` | `xszs=info,tower_http=info` | 控制日志级别；不要开启会泄露敏感数据的临时调试日志 |

管理员 username 规范化为 3–64 bytes 的小写 ASCII，首尾字母数字，中间允许 `[a-z0-9._-]`；密码为 12–1024 bytes 且不含 ASCII 控制字符。手机使用实例授权码，管理员凭据仅用于管理页面。

## 4. 配置 HTTPS

由代理提供 TLS，服务默认只监听 `127.0.0.1:8080`。最小 Caddy 站点配置：

```caddyfile
media.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

将示例域名换成实际域名并验证证书。`TRUSTED_PROXY_CIDRS` 只包含真实直连代理；代理覆盖访客提供的转发头并传递 HTTPS 语义。防火墙限制业务流量经代理进入。

## 5. 初始化并启动

以下命令由 systemd 为服务用户加载私有环境。`init` 读取配置中的初始管理员密码：

```sh
sudo systemd-run --wait --collect -p User=xszs -p Group=xszs \
  -p EnvironmentFile=/etc/isarmg/xszs.env \
  /opt/isarmg/xszs/releases/1.0.1/bin/xszs init
sudo /opt/isarmg/xszs/releases/1.0.1/scripts/start-server-wsl.sh
```

`init` 只用于全新状态，普通运行不会创建数据库或重置管理员。已有状态使用 `config validate`。
启动脚本复核发行、unit、配置和部署指针，然后启用并启动 `xszs.service`。

## 6. 验证第一份上传

```sh
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/readyz
/opt/isarmg/xszs/releases/1.0.1/scripts/verify-server-wsl.sh
```

本机检查应成功，验收脚本默认输出 `health=204 admin_page=200`。自定义监听时可为脚本设置 `XSZS_VERIFY_URL` 和 `XSZS_VERIFY_FORWARDED_PROTO`。

接着从真实 HTTPS 域名打开 `/admin`，登录并新建备份实例。手机 xszc 使用实例授权码配对，上传一份测试照片或视频，核对提交与读取结果。完成这一步后才确认代理、认证和媒体路径都可用。

## 日常运行

```sh
sudo systemctl status xszs.service --no-pager --full
sudo journalctl -u xszs.service --since today --no-pager
sudo systemctl stop xszs.service
```

上面第三条用于停服维护，按需要执行。`scripts/run-server-wsl.sh` 会启动已有部署并跟随日志；Ctrl+C 仅停止日志查看。
关注磁盘/inode、上传错误、重启和证书到期。非空 `METRICS_TOKEN` 启用受独立 Bearer Token 保护的 `/metrics`。

### 离线诊断

`doctor` 和 `reconcile scan` 需要停服并确认进程退出。用同一环境、同一服务用户执行：

```sh
sudo systemd-run --wait --collect -p User=xszs -p Group=xszs \
  -p EnvironmentFile=/etc/isarmg/xszs.env \
  /opt/isarmg/xszs/releases/1.0.1/bin/xszs doctor
```

Doctor 检查当前结构、数据库完整性、对象 Hash、上传恢复及存储。若诊断确认有待处理提交或回收对象，使用相同前缀把最后的 `doctor` 换为 `reconcile scan`；它会推进已记录的提交和物理回收。完成后重新检查并启动。
诊断预算为最多 100,000 项、64 MiB 索引/路径、64 层目录、1 TiB Hash 读取、600 秒总时限及三秒 SQLite 查询；超限会报错。

## 常见问题

| 症状 | 检查与处理 |
|---|---|
| 发行校验失败 | 核对摘要、源码身份、架构、文件权限和归档完整性 |
| 目标或 unit 已存在 | 保留已有部署，确认它的来源；安装器只处理新目标 |
| 初始秘密审阅未完成 | 审阅三个独立秘密，删除标记并保持配置权限 |
| 服务启动后退出 | 查看 Journal 第一条错误，检查路径、权限、当前结构和密钥 |
| 存活正常而 readiness 503 | 检查挂载、数据库、空间、inode 和存储权限 |
| HTTPS 或登录失败 | 检查证书、真实代理地址、username、Cookie 和主机时间 |
| 上传失败 | 检查手机权限和队列、分块大小、并发、代理限制和存储空间 |

数据结构错误时保留原数据并停止写入调查；手改元数据无法修复结构。
删除 API 返回 `202` 表示物理回收仍在处理中，`204` 表示完成；检查协调状态后再处理。
分享故障时提供脱敏错误、时间、请求 ID 和程序身份，生产媒体、数据库及秘密保留在受控环境。

## 包内容与进一步阅读

归档包含 `bin/xszs`、配置样例、四个安装/运行/验证脚本、systemd unit、`share/web-assets.json`、manifest、LICENSE、本手册和 `docs/feature-inventory-and-tradeoffs.md`。Web 字节嵌入程序。
完整源码、开发与 API 文档见 [xszs 文档入口](https://github.com/isarmg/xszs/blob/main/docs/README.md)；使用时核对与 `source_revision` 对应的提交。
