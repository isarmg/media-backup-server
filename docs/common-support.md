# xszs 的公共支撑职责与单体依赖

本项目属于 **Server**，Rust 公共支撑只依赖 **`xcss` 一个包**。公共代码已物理迁入该包的 `src/` 内部模块；下游没有继续依赖原来的子 crate，也没有通过一个外壳包间接安装它们。

| 公共包 | 唯一使用方 | 平台范围 | 分发方式 |
|---|---|---|---|
| `xcss` | Server | Linux x86_64 / AMD64，GNU libc，`x86_64-unknown-linux-gnu` | 一个 Rust Cargo package |
| `@xcss/web` | Server 的管理 Web 构建 | 构建机为 Linux x64 / glibc；生成页面由浏览器访问 | 一个 npm 包、一个 `xcss-web-1.0.0.tgz` |

包名、仓库名、软件版本和内部模块是不同概念。`xcss::log` 或 `xcss::server_runtime` 表示同一个包内的模块，不代表一个独立依赖包。版本号使用 `1.0.0`；产品维护的数据合同、UUIDv4、IPv4/IPv6、第三方软件版本及系统 API 名称保持其真实含义。

## 依赖如何固定

根或子目录 `Cargo.toml` 固定官方 Git URL、完整 40 位 `rev` 和 `version = "=1.0.0"`；`Cargo.lock` 固定该源与依赖摘要。两者必须一起更新。临时本地覆盖仅用于联调，正式 CI 与发行构建使用固定的官方源码。

消费产品的业务协议包可以共享严格的数据类型，但必须保持独立：Client 依赖协议类型时，依赖图也不能间接带入 `xcss`、Server HTTP、管理员后台或 Server 生命周期实现。

## Server 集成

Rust 使用 `xcss::contracts`、`xcss::config`、`xcss::log`、`xcss::sqlite`、`xcss::server_runtime`、`xcss::admin_core` 等包内模块。Server 整包编译明确拒绝非 Linux AMD64 GNU 目标；不能通过关闭 feature 使用它构建其他平台的 Client。

管理 Web 只声明一个公共依赖：

```json
"@xcss/web": "https://github.com/isarmg/xcss/releases/download/v1.0.0/xcss-web-1.0.0.tgz"
```

认证、HTTP、界面、字体和工具链分别由 `@xcss/web/contracts`、`@xcss/web/http-client`、`@xcss/web/admin-web`、`@xcss/web/admin-shell`、`@xcss/web/admin-ui`、`@xcss/web/design-tokens`、`@xcss/web/web-fonts` 和 `@xcss/web/web-toolchain` 子路径提供。它们共享一个版本、一次下载和同一个锁文件完整性摘要；不会作为八个独立包发布。

在已安装本项目规定的 Rust 与 Node 工具链后，从仓库根目录执行：

```bash
npm --prefix web ci
web/node_modules/.bin/xcss-build-server --config xcss-web-build.json --mode development --no-install
cargo tree --locked -e normal
```

- `npm ci` 按锁文件重新安装精确构建输入，同时校验下载完整性；不能用来源不明的 CSS 或相邻仓库文件替代。
- `xcss-build-server` 读取 `xcss-web-build.json`，先构建管理 Web，再构建 Linux AMD64 Rust 可执行程序，并校验实际二进制的内嵌资源清单；`--no-install` 复用上一步已经安装的锁定依赖。
- `cargo tree --locked -e normal` 查看实际运行依赖，查看公共包 `xcss` 及产品自身运行依赖。开发验证依赖通过项目原有测试入口另外检查。

生成的浏览器页面和内嵌资源属于 Server 产品。运行机器按本项目部署文档安装服务；无需在生产数据目录另装 npm 包或启动公共库服务。

## 更新与排查

公共包升级需要一起更新源码 revision、Cargo 锁、调用代码、平台门禁和发行输入。Server 还需更新单个 Web tarball 的 URL 与真实 SHA-512 integrity，并重新构建整个内嵌资源快照。

出现找不到包内模块、旧子包、未匹配的 Git revision 或锁文件完整性错误时，先核对上述清单是否来自同一次发布。不能加入旧名称 alias 或跳过平台/完整性检查。最终是否可部署，以本项目当前源码的 CI 和实际发行制品验收为准；历史小包的检查结果不能代替单体重构后的验收。
