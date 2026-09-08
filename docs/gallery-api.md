# 图库查询、快照与媒体读取扩展

本次追加 `/v2` 图库接口，不改变现有上传请求/响应、存储编码、数据库 schema 或当前产品状态身份。
它们与 Client 0.4.0 的第三阶段功能配套；现有发行物不会自动包含工作区中的新增接口。
全部接口经过原有设备/API key 鉴权，并以实际 account_id 限定查询；管理员会话不是媒体访问凭据。

## 全库时间线

`GET /v2/timeline` 保留 cursor、limit、trashed、favorite、archived、album_id、tag_id，追加：

| 参数 | 语义 |
|---|---|
| media_kind | photo、video、other；省略表示全部 |
| from_ms | source_created_at_ms >= 此毫秒时间戳 |
| to_ms | source_created_at_ms < 此毫秒时间戳；同时传起止时必须 from_ms < to_ms |
| device_id | 限定当前账户的某台来源设备；其他账户设备不返回数据 |

筛选在 SQL 中执行，使用现有时间 + UUID 游标、默认 100、最大 250。客户端改变任何筛选必须重置游标。
资产、资源、标签在同一读事务中批量查询；单页为一次 ID 查询和三次批量摘要查询，消除逐资产查询。
`GET /v2/devices` 返回当前账户的 `{device_id,name,platform}` 数组。

## 首次缓存与增量

1. 请求 `GET /v2/sync/head`，保存 `{sequence,snapshot_protocol:"watermark-before-uuid-walk-v1"}`。
2. 以该 sequence 创建空缓存；请求 `GET /v2/library/snapshot?limit=100`，随后按 next_cursor（UUID）继续。
3. 快照按 UUID 升序，包含普通资产和回收站；每一页 items 与 next_cursor 在客户端同事务保存。
4. 最后一页 next_cursor=null 后，请求现有 `GET /v2/sync?after=<第1步sequence>`。
5. 对资产事件读取最新 `/v2/assets/{id}`；仅 404 作为不存在，其他错误整页重试。最新状态可覆盖多个历史事件。
6. 客户端在同一事务里更新/删除资产、使分页成员缓存失效、推进 next_sequence；空页不虚构更大的序号。

快照不是跨请求持有的 SQLite 长事务。水位必须在 UUID 遍历之前捕获：遍历期间插入到游标之前的资产、
已经读过资产的更新、永久删除都由随后重放的事件补齐。不能沿用以前只推进但没有应用事件的旧游标。
批量标签成员变动同时为旧/新成员集合生成 asset upsert 事件；相册变动使查询页成员缓存失效。

## 资源内容与视频

`GET /v2/resources/{id}/content` 和 HEAD 返回 ETag（内容 BLAKE3）、Accept-Ranges: bytes。

- 单段显式、开放尾部、后缀 Range 返回 206、Content-Range 和准确 Content-Length。
- 无效或不可满足的范围（包括多段）返回 416 和 `Content-Range: bytes */<size>`。
- 匹配 If-None-Match 返回 304；If-Range 不匹配时忽略 Range 并返回完整 200。
- 内容从安全打开的 blob 流式 seek/take；原件长度须与记录一致。编码仍为 plain-v1。
- 客户端播放器必须在每次分段请求中鉴权，验证 HTTPS 同源，禁止重定向，关闭时取消加载。

## 派生预览

`GET /v2/resources/{id}/preview` 对支持的图片生成最长边 1600 像素 JPEG（质量 85），应用原方向。
JPEG/PNG/WebP/TIFF/GIF 由固定版本图像库解码；视频或不支持的格式/尺寸返回 415，客户端可回退原件。
派生图不是备份资源，不新增/替换 resources 或 blobs 记录；删除派生缓存不会影响恢复。

服务端生成并发 2、解码分配预算 256 MiB、单边最大 20000、派生内存缓存 64 MiB。
缓存键包含账户、内容哈希和派生版本；先鉴权再读缓存。返回私有缓存策略与独立 ETag，支持 If-None-Match。
原始资源的 MIME、内容哈希和字节不因生成预览改变。

## 验证

真实 SQLite/HTTP 集成测试覆盖全库筛选、批量摘要与单项详情一致、分页快照、快照期间插入、
水位后事件、标签旧成员失效、设备隔离、跨账户内容/预览拒绝、HEAD、三种 Range、416、
条件请求、预览尺寸与原件不变、视频预览 415；移动播放/编码兼容性还需手机验收。
