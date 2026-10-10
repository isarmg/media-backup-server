# xszs 开发与验证

构建使用 Rust 1.99.0、Node.js 26.7.0 和 Linux AMD64 GNU 目标。前端和服务端由
`xcss-build-server` 的共同入口构建，打包命令见[运维文档](operations.md#2-构建与验证发行归档)。

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
