# Media Backup Server

本仓库拥有 Server、管理 Web 和唯一的 `media-backup-protocol` 源码。
移动端通过完整 Git 提交固定该协议依赖；Server 不编译移动端队列、加密核心或 FFI。
Server 发行身份仅绑定 Server 的源码、版本、API、存储、Schema 和 Web 资源，不绑定或附带移动 ABI 头文件。

Client 仓库：https://github.com/isarmg/media-backup-client 。其沿用原产品仓库历史与 Android 签名环境；
本 Server 仓库没有复制签名 Secrets，工作流明确禁止引用移动签名环境或 Secrets。
移动端文档与安装流程请以 Client 仓库为准。本次不改写任何历史标签，也不声明新版本已正式发布。
