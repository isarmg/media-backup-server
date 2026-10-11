# xszs

## 项目简要介绍

自托管的照片与视频备份服务。Rust 服务端接收独立 xszc 移动客户端的数据，并提供内置管理页面。

## 项目功能

- 移动设备配对、账号与备份实例管理
- 分块上传、媒体索引、图库查询和文件交付
- 运行状态、管理员账号与访问凭据管理

## 适用平台

服务端仅支持 Linux AMD64 GNU（`x86_64-unknown-linux-gnu`），需要 systemd、Python 3.11+、GNU 工具及 HTTPS 反向代理。Android/iOS 客户端由独立 xszc 项目提供。

## 如何快速部署

从 [下载页](https://github.com/isarmg/xszs/releases) 下载 Linux 归档及同版 `SHA256SUMS`，在全新主机目录执行：

```sh
sha256sum --check SHA256SUMS
tar -xzf xszs-1.0.1-x86_64-unknown-linux-gnu.tar.gz
cd xszs-1.0.1-x86_64-unknown-linux-gnu
./bin/xszs release-verify "$PWD"
sudo ./scripts/setup-wsl.sh
sudoedit /etc/isarmg/xszs.env
```

审阅管理员密码、`XSZS_CREDENTIALS_KEY` 与 `METRICS_TOKEN`，删除已确认的 `INITIAL-SECRETS-MUST-BE-REPLACED` 标记。默认监听 `127.0.0.1:8080`；配置 HTTPS 网关后显式初始化并启动：

```sh
sudo systemd-run --wait --collect -p User=xszs -p Group=xszs \
  -p EnvironmentFile=/etc/isarmg/xszs.env \
  /opt/isarmg/xszs/releases/1.0.1/bin/xszs init
sudo /opt/isarmg/xszs/releases/1.0.1/scripts/start-server-wsl.sh
```

安装器不覆盖已有发行目录或 systemd unit，普通启动不创建数据库。媒体按明文字节保存，主机需提供存储加密和最小权限。

## 如何编译部署

在 Linux AMD64 的干净源码目录准备 Rust 1.99.0、Node.js 26.7.0 与 C 编译工具：

```sh
rustup target add --toolchain 1.99.0 x86_64-unknown-linux-gnu
revision="$(git rev-parse HEAD)"
npm ci --prefix web
web/node_modules/.bin/xcss-build-server --config "$PWD/xcss-web-build.json" \
  --mode release --no-install --source-revision "$revision"
mkdir -p "$PWD/dist"
./scripts/build-server-release.sh \
  "$PWD/target/x86_64-unknown-linux-gnu/release/xszs" "$revision" "$PWD/dist"
```

输出 `dist/xszs-1.0.1-x86_64-unknown-linux-gnu.tar.gz`，按上面的安装、配置和初始化步骤部署。

[详细文档](docs/README.md)
