# Media Backup Server 文档总览

本文档集描述 Server `0.3.19` 的当前代码。服务端数据库合同独立保持为 `media-backup` `0.3.0`、
schema revision 5；软件版本、数据库版本和 Client 移动状态版本不是同一个编号。

| 文档 | 适用对象 | 内容 |
|---|---|---|
| [仓库边界](repository-boundary.md) | 开发者 | Server、Web、protocol 与独立 Client 仓库的职责 |
| [功能与取舍](feature-inventory-and-tradeoffs.md) | 设计与评审 | 当前功能、风险、删除影响和验证边界 |
| [接口消费者](interface-consumers.md) | API 与 UI 开发者 | 数据面、管理面及各接口的实际消费者 |
| [图库 API](gallery-api.md) | 移动端与 Server 开发者 | 筛选、快照、增量、Range 与派生预览 |
| [运维手册](operations.md) | 发布与值班人员 | 构建、安装、配置、诊断、备份缺口和发布门禁 |
| [发行包手册](server-release-readme.md) | 部署人员 | 已构建归档的校验、安装、运行和退役 |
| [账号设置](account-settings.md) | 管理员 | 当前管理员账号修改与会话失效语义 |

历史行为只在 `releases/` 中说明；排查当前系统时不要把旧发行说明当作现行合同。Android/iOS 构建、
移动队列和签名资料请查阅独立的
[media-backup-client](https://github.com/isarmg/media-backup-client) 文档。
