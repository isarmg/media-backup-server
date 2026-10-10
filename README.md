# xszs

当前工作树为 `1.0.0` 发行候选；正式源码、标签与资产以通过 CI 的精准 Source 和 Release manifest 为准。

当前启动入口和初始化边界见 [服务命令](docs/cli.md)。部署须先显式 `init`，再 `run`；配置验证和状态查询失败会返回非零退出码。

xszs `1.0.0` 是自托管的照片与视频备份服务。Rust/Axum 服务端负责移动设备配对、分块上传、媒体索引和文件交付，内置管理 Web 用于管理备份实例、账号和运行状态。

正式 Server 仅支持 Linux AMD64 GNU（`x86_64-unknown-linux-gnu`）。Android/iOS 应用位于独立的 [xszc](https://github.com/isarmg/xszc) 仓库。

设备侧从安装、配对/重新配对到服务或后台任务管理、诊断与卸载，见独立 [Client 分平台部署指南](https://github.com/isarmg/xszc/blob/main/docs/platform-setup.md)。

## 配置概览

从模板创建生产环境文件，并生成独立的凭据加密密钥：

```sh
sudo install -d -m 0750 /etc/isarmg
sudo install -m 0600 config/xszs.env.example /etc/isarmg/xszs.env
openssl rand -base64 32
sudoedit /etc/isarmg/xszs.env
```

至少设置数据库、数据目录、管理员密码、`XSZS_CREDENTIALS_KEY` 和 `METRICS_TOKEN`。默认模板监听 `127.0.0.1:8080`，生产环境应由 HTTPS 反向代理对外提供服务。

构建并验证发行包：

```sh
revision="$(git rev-parse HEAD)"
npm ci --prefix web
npm run build --prefix web
XSZS_SOURCE_REVISION="$revision" cargo build --release --locked -p xszs \
  --target x86_64-unknown-linux-gnu
mkdir -p "$PWD/dist"
./scripts/build-server-release.sh \
  "$PWD/target/x86_64-unknown-linux-gnu/release/xszs" \
  "$revision" "$PWD/dist"
./scripts/test-deployment.sh \
  "$PWD/dist/xszs-1.0.0-x86_64-unknown-linux-gnu.tar.gz"
```

发行树安装、启动、账号维护、备份和恢复步骤见[运维文档](docs/operations.md)。

## 开发验证

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## 文档

- [文档总览](docs/README.md)
- [仓库与 Client/Server 边界](docs/repository-boundary.md)
- [接口消费者边界](docs/interface-consumers.md)
- [功能范围与取舍](docs/feature-inventory-and-tradeoffs.md)
- [部署与运维](docs/operations.md)

代码采用 [Apache License 2.0](LICENSE)。

当前发布版本：**1.0.0**。参见 [1.0.0 发布说明](docs/releases/1.0.0.md)和[项目命名](docs/naming.md)。

公共支撑的职责、单体依赖、平台边界与验证方法见[公共支撑说明](docs/common-support.md)。
