# 接口与消费者边界

本表记录容易被误判为“有后端、无界面”的业务接口。管理员浏览器 Session 与媒体账户的设备
Bearer Token/API Key 属于不同身份域；消费者不得代持另一身份域的凭据来补界面。

| 接口 | 当前消费者 | 产品界面决策 |
|---|---|---|
| `DELETE /v2/assets/{asset_id}` | Android、iOS 回收站详情 | 移动端提供二次确认；`202` 表示逻辑删除已提交且物理回收继续协调 |
| `POST/DELETE /v2/tags/{tag_id}/assets/{asset_id}` | Android、iOS 资产详情 | 移动端提供单资产添加和移除，适合日常整理 |
| `PUT /v2/tags/{tag_id}/assets` | 账户 API、自动化工具 | 保留为全量成员设置接口；移动端不提供批量覆盖，避免把多选误操作变成全量替换 |
| `GET /v2/duplicates` | Android、iOS 重复项分组入口 | 只展示分组和资产，由用户决定处理；不自动删除 |
| `GET/POST /v2/api-keys`、`DELETE /v2/api-keys/{id}` | 账户 API、自动化工具 | 不接入管理员 Web 或移动图库；API Key 原值仅在创建响应出现一次 |
| `GET /v2/audit-events` | 账户 API、诊断工具 | 不合并进 `/api/v2/admin/logs`；调用者必须持有对应账户凭据 |
| `/api/v2/admin/*` | 管理 Web | 只管理 Server 实例、配额、路径和管理员审计，不读取媒体账户凭据 |

“账户 API、自动化工具”是明确支持的接口面，不代表管理员 Web 的缺口。若以后增加账户设置客户端，
它必须以账户身份独立登录，并为 API Key 一次性展示、撤销和账户审计分页提供专门流程；不能把设备
Token 或 API Key 写入管理员浏览器存储。
