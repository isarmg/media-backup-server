# Media Backup 照片与视频备份

本仓库为 Media Backup Server `0.3.8` 开发版，仅包含 Rust 服务端、协议和管理 Web。
Android/iOS 客户端、共享队列核心和 FFI 位于独立的 [Client 仓库](https://github.com/isarmg/media-backup-client)。移动端通过 HTTPS 上传设备原始媒体和设备生成的缩略图；服务端使用
SQLite 保存账户、图库组织和同步状态，并以 `plain-v1` 保存与设备原文件一致的未加密字节。

本项目只实现当前版本。服务端创建并只接受 Media Backup `0.3.0` 数据格式（Schema revision 4），软件版本 `0.3.8`；移动端状态合同由 Client 仓库规定。
不属于当前身份的数据库和凭据一律拒绝，产品仓库也不提供
迁移、备份和恢复命令。离线任务只能由 `sarmg-upgrade` 的精确支持矩阵执行；该工具目前尚不支持
Media Backup `0.3.0` / revision 4，不能用旧适配器处理当前状态。

## 组成

```text
crates/server       Rust/Axum 服务端、管理页和运维命令
crates/protocol     服务端与移动端共享的数据协议
web/                React 19 + TypeScript strict + Vite 7 管理客户端
config/                     可提交配置样例；生产 Secret 位于源码树外
deploy/                     systemd 源单元；发行时映射到 systemd/
scripts/                    构建、发行、部署和契约门禁
```

## 快速验证

```bash
./scripts/check-workflow-supply-chain.sh
npm ci --prefix web
npm run build --prefix web
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Web 使用 Node `26.7.0`；`build` 会先执行 `check:foundation`，核对 `.node-version`、engine、精确
React/Vite/TypeScript 版本、Foundation 依赖、lockfile、CSS 摘要和 `data-sarmg-scope`。Server 使用
`rust-toolchain.toml` 固定 Rust `1.98.0`，Rust 编译前必须先生成 `web/dist`。

服务端开发构建可以在显式回环开发配置下运行；正式环境必须使用不可变发行归档，并由 Caddy 或
Nginx 在可信代理边界终止 TLS。移动业务 API 只位于 `/v2`；浏览器管理员认证只位于
`/api/v2/auth/login|session|logout`，管理业务只位于 `/api/v2/admin/*`。正式 Server 的编译目标、
归档 ELF、运行主机和 systemd 条件都固定为 `x86_64-unknown-linux-gnu`/Linux x86_64，不提供 ARM Server
fallback。完整步骤见运维文档。

Server 管理身份采用 Foundation 当前 `username` 合同：登录只接受 `{username,password}`，成功 Session
恰为 `{authenticated,user_id,username,role:"admin",csrf_token}`。移动客户端不使用备份账户密码：管理员在备份账户下创建实例后，把该实例唯一的长期授权码交给 Android/iOS 配对。授权码以信封密文保存且可查看/轮换，轮换后客户端必须重新配对。这里只支持新的当前 Schema，不包含旧密码 bootstrap 或进程内迁移兼容。

## 文档

- [仓库边界](docs/repository-boundary.md)
- [完整功能与取舍清单](docs/feature-inventory-and-tradeoffs.md)
- [部署、诊断、安全与发布运维](docs/operations.md)

## 许可证

第一方代码、文档和资源采用 [Apache License 2.0](LICENSE)。项目只参考其他照片管理产品的公开行为
和架构思想，不复制其代码、资源、数据库结构或生成物。

图库查询、增量快照、中等预览与视频分段读取的新增接口见 [图库 API 扩展](docs/gallery-api.md)。

账号修改方法见 [账号设置](docs/account-settings.md)。
