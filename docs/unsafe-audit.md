# Rust unsafe 审查

审查日期：2026-10-08；对象：xszs 1.0.0 发行候选。正式发行身份由精准 Source 的 CI 与 Release 资产证明。

## 结论与约束

产品 Rust 源码、协议、构建脚本、示例与测试中没有 `unsafe` 块、unsafe 函数/实现或 unsafe extern 定义，因此没有需要保留的产品 unsafe。根 `Cargo.toml` 定义 `[workspace.lints.rust] unsafe_code = "forbid"`，两个内部包均继承，后续不能通过模块级 allow 引入无依据的 unsafe。

该结论针对自研产品源码，不表示第三方依赖或上游 Foundation 不使用 unsafe。操作系统、数据库和密钥能力通过现有安全 API 使用；依赖中的平台/FFI 边界仍由对应上游负责审查和验证，不复制它们的实现。

## 工程证据

审查范围包含 `crates/**/*.rs`、构建脚本与 `scripts/*.rs`；代码搜索用于定位，Cargo workspace lint 用于编译约束。Rust 1.99.0 格式检查和协议测试已执行；Linux Server 验证的实际范围单独记入候选发行说明，不能把 macOS 上的协议测试当作 Linux 文件系统、服务锁或发行物运行验收。

媒体文件继续使用现有 `rustix` 的根目录描述符与 `openat2`，保留 BENEATH/NO_SYMLINKS、普通文件身份与原路径复核。发行校验中的文件打开也使用 rustix 安全 API，保留 NOFOLLOW/CLOEXEC/NONBLOCK 与打开前后的 mode、owner、inode/大小检查；不再仅为获取 O_* 常量直接依赖 libc。未削弱符号链接、硬链接或读取上限校验。
