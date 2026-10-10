# 配置参考

生产安装使用 root 拥有、权限 `0600` 的 `/etc/isarmg/xszs.env`。先用安装器生成，再审阅实际值。

## 配置项

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

## 管理员与设备凭据

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

## HTTPS 代理

生产流量应为：

```text
浏览器或移动客户端
        │ HTTPS
        ▼
可信反向代理
        │ HTTP，仅同机 loopback 或受控私网
        ▼
127.0.0.1:8080 xszs
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

严格 JSON、CLI 优先级和只读校验见[服务命令](cli.md)。同一服务始终使用同一组数据库、媒体目录和加密密钥。修改后在维护窗口重启，并检查 `status` 与真实 HTTPS 登录。
