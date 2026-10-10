# xszs 的公共依赖

本服务通过一个 Rust 包 `xcss` 使用公共配置、日志、SQLite、认证和生命周期模块；管理 Web 通过一个 `@xcss/web` 包使用组件、字体和工具链。

| 依赖 | 用途 | 平台 |
|---|---|---|
| `xcss` | Rust Server 公共模块 | Linux AMD64 GNU，`x86_64-unknown-linux-gnu` |
| `@xcss/web` | 构建管理页面和内嵌资源 | 当前 Linux x64/glibc 构建工具链，输出供浏览器使用 |

## 固定输入

Cargo manifest 指定官方 Git URL、完整 revision 和精确 `=1.0.0`，Cargo.lock 固定依赖解析结果。
Web 的 package.json 依赖条目如下，合并到项目自身的 dependencies 中：

```json
{
  "dependencies": {
    "@xcss/web": "https://github.com/isarmg/xcss/releases/download/v1.0.0/xcss-web-1.0.0.tgz"
  }
}
```

认证、HTTP、Shell、UI、字体和构建工具通过同包的 `contracts`、`http-client`、`admin-web`、`admin-shell`、`admin-ui`、`design-tokens`、`web-fonts`、`web-toolchain` 子路径导入，共享版本和锁文件完整性摘要。

## 构建和检查

准备项目固定的 Rust、Node 工具链后，在仓库根目录执行：

```sh
npm ci --prefix web
web/node_modules/.bin/xcss-build-server --config xcss-web-build.json --mode development --no-install
cargo tree --locked -e normal
```

`npm ci` 校验并安装锁定输入。共同构建器先生成 Web，再构建 Server，最后比较实际程序中的资源清单和刚生成的资源。
`cargo tree` 用于核对真实运行依赖。生产运行使用内嵌资源，公共包属于编译输入。

## 更新依赖

同时更新源码 revision、manifest、锁文件、调用代码和测试；Web 还需核对 tarball URL 与真实 SHA-512 integrity，并重新生成内嵌资源。
遇到找不到模块或完整性错误，先对照同一版本的来源和锁文件，再按项目开发文档构建验证。最终源码、目标平台和实际发行物分别记录验收结果。
