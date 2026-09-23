# Media Backup Server

本仓库拥有 Server、管理 Web 和唯一的 `media-backup-protocol` 源码。
移动端通过完整 Git 提交固定该协议依赖；Server 不编译移动端队列、加密核心或 FFI。
Server 发行身份仅绑定 Server 的源码、版本、API、存储、Schema 和 Web 资源，不绑定或附带移动 ABI 头文件。

Client 仓库：https://github.com/isarmg/media-backup-client 。Android/iOS 构建、签名、安装和客户端文档由该仓库维护。
Server 工作流禁止引用移动签名环境或 Secrets。正式发布状态以本仓库不可变发行标签和制品为准。
