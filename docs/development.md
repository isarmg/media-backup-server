# xszs 开发与验证

构建使用 Rust 1.99.0、Node.js 26.7.0 和 Linux AMD64 GNU 目标。前端和服务端由
`xcss-build-server` 的共同入口构建，打包命令见[下方构建步骤](#构建发行归档)。

## 开发验证

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

构建归档后执行部署验收：

```sh
./scripts/test-deployment.sh "$PWD/dist/xszs-1.0.0-x86_64-unknown-linux-gnu.tar.gz"
```

核心命令、显式初始化、只读验证与非零失败语义见[服务命令](cli.md)。移动端安装、配对、
后台任务管理、诊断与卸载见 [xszc 平台指南](https://github.com/isarmg/xszc/blob/main/docs/platform-setup.md)。
代码采用 [Apache License 2.0](../LICENSE)。


## 构建发行归档

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
revision、target、`v1` 移动 API、`plain-v1`、Schema、Web 与全树文件权限/大小/SHA-256；
额外文件、链接、特殊文件或硬链接别名均失败。

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

`npm run test:browser --prefix web` 对实际 dist 运行 Chromium/Firefox 验收，覆盖实例原子创建与配对、
失败重试、内部数据归属与管理员账户入口隔离、无平台管理员面板、字体资产、键盘焦点及移动明暗主题 WCAG AA。首次运行先在
`web` 执行 `npx playwright install --with-deps chromium firefox`。

当前 Server Rust 固定 xcss `=1.0.2` / `3f751196615edd9f7fda2d76a5aa90f9f42586dc`；一个 @xcss/web 包使用
xcss 1.0.2 正式 Release tarball 与 lockfile integrity，不依赖相邻工作区。本仓库 CI 验证 Server、Web 与发行
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
