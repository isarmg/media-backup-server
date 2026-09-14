# Media Backup 完整功能与取舍清单

本文按 Media Backup `0.3.11` 当前工作树盘点 Server、React 管理 Web、共享 Rust Client、Android、iOS、
协议、存储和交付闭包。代码、两个 `current_schema.sql`、移动 epoch、FFI header 和发行 manifest 是最终
事实源；本文不把规划中的能力写成已实现功能。

## 1. 阅读规则

### 1.1 分类

| 分类 | 含义 |
|---|---|
| 核心 | 直接构成“移动设备把照片/视频可靠备份到自托管 Server 并可查询恢复”的主目标 |
| 保障 | 保护认证、账户隔离、完整性、崩溃一致性、资源上限或失败关闭 |
| 可选 | 只服务特定平台、部署或使用习惯，可在接受明确损失后删除 |
| 建议保留 | 不改变核心协议，但显著改善后台可靠性、管理或用户体验 |
| 开发运维 | 构建、验证、配置、诊断、发布、文档和供应链能力 |

### 1.2 复杂度

| 复杂度 | 含义 |
|---|---|
| 低 | 单一页面、字段或独立脚本，通常不改变持久状态 |
| 中 | 跨两个以上模块、语言或配置，需要成组修改 |
| 高 | 跨移动端、FFI、HTTP、Schema、文件系统或发行身份，不能局部删除 |

### 1.3 两类“用户”和两类 `role`

控制面只有 Administrator。`_sarmg_administrators` 表不保存 `role`，Administrator Session 的 `role:"admin"` 是
Foundation wire 常量；其身份键是 `_sarmg_administrators.username`。`accounts` 只是照片/视频归属的数据面
租户，不是低权限管理员，也不是客户端登录身份。管理员 username 即使与租户名称文字相同也不会关联。上传
`resources.role` 表示同一资产内的资源用途，例如 `primary` 或 `thumbnail`，也不是权限角色。删除或改名
任一概念时必须保持这三个命名空间彼此独立。

### 1.4 删除闭包

删除功能需同时检查：Rust crates、Server route/DTO、两个 SQLite Schema、Kotlin/Swift 宿主、C/JNI FFI、
React 管理页、配置与 systemd、发行 identity/manifest、CI/脚本、正反测试和中文文档。只隐藏按钮、只
停止调用或只删一个平台实现，都不能视为功能已从项目边界移除。

## 2. 产品定位、平台和目录

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| MED-P-001 | Android/iOS 把授权范围内的照片、视频和设备生成缩略图备份到自托管 Server | `clients/android`、`clients/ios`、`crates/server` | 核心 | 高 | 项目不再是完整移动媒体备份系统 | 两平台至少一条原始媒体+缩略图端到端 |
| MED-P-002 | Server 唯一支持 `x86_64-unknown-linux-gnu`，正式主机唯一为 Linux AMD64 | `sarmg-server-target`、server `build.rs`、release/systemd/scripts | 保障 | 高 | 会产生未经验证的 Server 平台制品 | 非目标编译、错误 ELF、错误 uname、systemd architecture |
| MED-P-003 | Android/iOS 客户端继续按各自平台架构构建；“Server 仅 AMD64”不限制移动 ABI | Android NDK targets、Apple targets | 核心 | 高 | 若误删移动架构，真机无法加载 Rust core | arm64 Android/iOS；模拟器；ABI/header 一致 |
| MED-P-004 | Server、移动 Client 和发行物只理解一个当前 `0.3.11` 身份 | metadata、mobile epoch、release identity | 保障 | 高 | 多代 parser 会扩大每次变更的状态矩阵 | 非当前 product/version/revision/epoch 零写入拒绝 |
| MED-P-005 | 产品不内置迁移、备份或恢复数据库命令；代际任务属于 `sarmg-upgrade` | Server CLI、Client open path | 保障 | 高 | 在线转换会把未知状态带入服务进程 | CLI 清单；Schema mismatch 只读失败 |
| MED-P-006 | 源配置统一位于顶层 `config/`，部署资产位于 `deploy/`，客户端位于 `clients/` | 仓库目录 | 开发运维 | 低 | 事实源散落，脚本和文档容易引用不同文件 | `rg` 不引用已移除目录；release 仍可重映射到 `systemd/` |
| MED-P-007 | React/Vite 管理客户端位于 `web/`；Android/iOS 原生客户端并列 | 目录结构、workspace scripts | 开发运维 | 低 | 客户端代码位置不一致，维护人员难以识别边界 | README、CI、构建脚本使用统一路径 |
| MED-P-008 | 原始媒体在 Server 使用 `plain-v1` 明文字节，传输机密性依赖 HTTPS | `StorageEncoding::PlainV1`、`crates/crypto` | 核心 | 高 | 改成端到端密文会重写缩略图、恢复、去重和密钥生命周期 | byte-for-byte round trip；HTTP 明文直连不得公网暴露 |
| MED-P-009 | Server Rust 和八个 Web 包已固定正式 Foundation 0.8.1 的完整 Git revision、Release URL 与 lock integrity，无相邻工作区来源 | Cargo、八个 `@sarmg/*` 依赖、manifest/lock | 保障 | 高 | 平台行为分叉；独立构建通过不代表主分支改动已纳入产品 Release | [独立 CI 与消费者证据](https://github.com/isarmg/sarmg-foundation-server/blob/main/consumers/axum-0.7.0-evidence.md)；后续更新仍须复验锁图和发行身份 |

## 3. 身份、认证与请求边界

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| MED-A-001 | 控制面只有 Administrator，平台表由 Foundation Composer 生成 | `_sarmg_administrators`、Admin Core | 核心 | 高 | 删除认证会公开备份实例与路径 | Session 固定 admin；无产品管理员表 |
| MED-A-002 | 仅无管理员时使用 BOOTSTRAP_ADMIN_USERNAME/PASSWORD 初始化；已有身份不被环境覆盖 | `build_state`、Admin Core | 保障 | 高 | 环境变量不能隐式重置账户 | 空库要求密码；已有管理员保持不变 |
| MED-A-003 | Administrator username 使用 Foundation 唯一 canonical 规则：登录 candidate 1..64 printable ASCII，经 trim ASCII + lowercase 后必须为 3..64 bytes、首尾字母数字、字符仅 `[a-z0-9._-]` | `normalize_administrator_username`、`require_canonical_administrator_username`、Schema CHECK | 保障 | 中 | 同一身份可用大小写/空白变体绕过限流或唯一约束，跨项目身份语义会漂移 | 大小写/首尾空白正例；`@`、Unicode、内部空白、控制字符、首尾分隔符和超长负例 |
| MED-A-004 | 管理员密码只接受 Foundation 当前 Argon2id 策略 | Admin Core、Admin Auth | 保障 | 高 | 产品不能另设 hash 分支 | PHC 参数与口令策略负例 |
| MED-A-005 | 三个浏览器认证入口精确为 `/api/v2/auth/login`、`/api/v2/auth/session`、`/api/v2/auth/logout` | Foundation path constants、`routes.rs` | 保障 | 中 | 路径漂移破坏共享客户端；别名扩大攻击面 | method/path 矩阵，合同外路径拒绝 |
| MED-A-006 | 登录 body 精确 username/password，Session 使用平台五字段合同 | Foundation Axum Adapter、Contracts | 保障 | 中 | 产品不能重解释认证结果 | strict DTO、401/403、ErrorEnvelope |
| MED-A-007 | 管理员登录来源为真实 socket peer；固定来源/账户失败预算和 Argon2 并发等待上限 | Foundation Admin Core | 保障 | 高 | 不能信任代理来源头或绕过平台预算 | 共享 Adapter 套件 |
| MED-A-008 | 未知账户执行当前 dummy hash | Foundation Admin Core | 保障 | 中 | 避免账户存在性时序差异 | dummy verification 测试 |
| MED-A-009 | 管理 Session/CSRF token 只以摘要存储 | Foundation Admin SQLite | 保障 | 高 | 产品不得另存明文 token | `_sarmg_admin_sessions` 当前 DDL |
| MED-A-010 | 固定每管理员 32、全局 1024 Session，空闲 30 分钟、绝对 12 小时 | Foundation Admin Policy | 保障 | 高 | 无产品 TTL 配置或分叉限额 | 过期、上限、last_seen 写入节流 |
| MED-A-011 | 恢复 Session 轮换当前 CSRF 摘要，客户端协调并发认证请求 | Foundation Admin Core、admin-web | 保障 | 中 | 不保留产品历史 CSRF 窗口 | 恢复、失效旧 token、并发客户端测试 |
| MED-A-012 | 写管理请求要求单个 CSRF、同源 Origin 与单一 Host/URI authority | Foundation Axum Adapter | 保障 | 高 | 禁止跨站和重复头歧义 | 共享 Axum/Hyper 套件 |
| MED-A-013 | 生产 Cookie 为 __Host-sarmg-media-backup-session，Secure/HttpOnly/SameSite=Strict/Path=/ | Foundation session_set_cookie | 保障 | 低 | 不提供产品 Cookie 别名 | 精确属性与 loopback 开发模式 |
| MED-A-015 | 数据面 accounts 与管理面 _sarmg_administrators 完全分离 | 产品移动授权、Foundation 管理授权 | 核心 | 高 | 同名账户不共享凭据或权限 | 移动 /v2 与管理 /api/v2 分域 |
| MED-A-016 | 管理员通过 `/api/v2/admin/instances` 只提交名称；Server 在同一事务中创建自动分配的内部 account 隔离边界、100 GiB 默认配额、device 和长期授权码，不暴露“先建用户再建实例”的流程。授权码保存为经实例 ID 绑定的信封密文和独立摘要；bootstrap 只接受待配对授权码并签发随机 Bearer token，轮换立即清除旧 token | `routes::bootstrap`、`admin.rs`、`crypto.rs`、`accounts`、`devices` | 核心 | 高 | 手机无法获得稳定设备身份；部分创建会留下孤立归属；明文库泄漏扩大 | strict name-only DTO、默认值、原子创建、正误授权码、取消/删除、轮换、disabled account、token digest、设备 audit |
| MED-A-017 | 移动业务 API 接受 device token 或未撤销 API Key，解析到同一 account/device context | `auth::require_auth` | 核心 | 高 | 无法授权上传，或两个 token 类型产生不同隔离语义 | device/API key、revoked、disabled、跨账户 |
| MED-A-018 | API Key 原值只在创建响应出现一次，库中存 hash/prefix，可列出和撤销 | `api_access.rs`、`api_keys` | 建议保留 | 高 | 自动化/额外客户端只能保存设备 token；明文保存会泄漏 | 创建、列表不含 token、撤销、last_used |
| MED-A-019 | `/metrics` 使用与其他身份分离的可选 Bearer `METRICS_TOKEN` | `metrics.rs`、Config | 保障 | 中 | 聚合容量可被公开，或监控被迫保存 Administrator Cookie | 无/错/对 token；空配置语义 |
| MED-A-020 | 可信代理解析从 socket peer 开始，最多 32 个转发 hop；未可信 peer 的头不生效 | `trusted_proxy.rs` | 保障 | 高 | 登录来源与 HTTPS 判断可被伪造 | trusted/untrusted、XFF 顺序、IPv4-mapped IPv6、超限 |
| MED-A-021 | 生产要求 HTTPS 语义；开发非 HTTPS 只允许显式 loopback | `require_secure_transport`、`validate_security_mode` | 保障 | 高 | Bearer、Cookie 和媒体元数据可能明文经过不可信网络 | direct/proxy HTTPS、XFP、development bind |
| MED-A-022 | 所有 HTTP 错误统一 Foundation `ErrorEnvelope`，429 保留 `Retry-After` | `error.rs`、`sarmg-error` | 保障 | 中 | React/移动端需要项目专用错误分支，或内部文本泄漏 | 400/401/403/404/409/429/500 exact keys |

## 4. 上传协议、配额与对象存储

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| MED-U-001 | 唯一移动 HTTP base 为 `/v2`，上传 DTO 使用 `deny_unknown_fields` | `crates/protocol`、`routes.rs` | 核心 | 高 | 三端 wire 漂移或歧义 JSON 被接受 | 全 DTO 正反 fixture、合同外 route |
| MED-U-002 | create upload 明确声明 source IDs、media kind、资源 role、文件名、MIME、时间、encoding、总 Hash/size、metadata 和 parts | `CreateUploadRequest` | 核心 | 高 | Server 无法重建资产/资源或校验完整性 | 每字段缺失/类型/unknown、完整 round trip |
| MED-U-003 | 只接受 `plain-v1` storage encoding | `StorageEncoding`、Schema CHECK | 保障 | 高 | 多 encoding 会要求下载、doctor 和恢复分支 | 非当前枚举拒绝；DB CHECK |
| MED-U-004 | manifest body 最大 64 KiB，普通 JSON 最大 256 KiB，metadata 最大 64 KiB | `routes::router`、`validate_upload_request` | 保障 | 中 | 超大 manifest/metadata 可耗尽内存和 DB | 边界、超限、非法 JSON、连接恢复 |
| MED-U-005 | parts 必须从 0 连续编号、非空、每块不超过 `MAX_PART_BYTES`，sizes 总和等于 content size | `validate_upload_request` | 保障 | 高 | 缺块、重叠或溢出会导致错误拼装 | 空文件、零块、gap、overflow、超大 part |
| MED-U-006 | 每块和完整对象使用 64 hex BLAKE3 | `crates/crypto`、storage/commit | 保障 | 高 | 传输损坏、错序和去重碰撞无法被当前流程识别 | 大小写 hex、错误 part、错误 full hash、篡改 |
| MED-U-007 | create upload 对相同账户内容先检查已有 blob，文件与 size/full hash 均匹配才去重 | `create_upload`、`inspect_object` | 核心 | 高 | 相同媒体重复占空间；只信 DB 会复用损坏文件 | 同账户正例、跨账户隔离、DB/file mismatch |
| MED-U-008 | 同 device/source_resource/full hash 的 active upload 返回原 upload 与缺块列表 | uploads lookup、`missing_parts` | 保障 | 高 | 移动网络重试会创建重复暂存和配额预留 | 同请求、不同 hash、已 commit、并发 create |
| MED-U-009 | quota 在创建时计入已用 blob、活跃上传预留和本次总 bytes；0 表示不限 | `ensure_quota` | 保障 | 高 | 单账户可占满共享盘，或并发 upload 越过额度 | 边界、并发预留、overflow、unlimited |
| MED-U-010 | part PUT 在写入前同时取得全局和每账户 admission permit，等待 30 秒后 429 | `UploadAdmission`、`put_part`、Config | 保障 | 高 | 单账户或并发客户端可占满分块写入资源 | create/complete 不经过该闸门；timeout/Retry-After、permit 回收 |
| MED-U-011 | part body 流式写入临时普通文件，实时限制 bytes、校验 size/hash 并 fsync | `LocalStorage::put_part` | 保障 | 高 | 整块缓冲耗内存；成功响应前数据可能未落盘 | 慢流、截断、超长、hash 错、取消、fsync 错 |
| MED-U-012 | 同一 part 的相同重试幂等成功，冲突字节不能覆盖已接收 winner | storage temp/no-clobber path | 保障 | 高 | 并发重试可互相覆盖或损坏 | 相同/不同内容并发、临时清理 |
| MED-U-013 | commit 状态显式为 receiving→commit_started→finalizing→committed，另有 unknown/failed | `upload_commit.rs`、Schema CHECK | 核心 | 高 | 崩溃点无法表达，重启会猜测最终文件/metadata | 每个 failpoint、合法字段组合、重启 |
| MED-U-014 | commit 重新读取每个 part、核对 Hash/size、拼装 stage、核对完整对象并 fsync | `storage::assemble`、`upload_commit` | 保障 | 高 | 仅信 received_at 会把损坏 part 发布为媒体 | part 篡改、缺失、full hash、stage fsync |
| MED-U-015 | final object 与 stage 位于同一受控文件系统，以 no-replace `linkat` 发布同一 inode、核对 device+inode 后 fsync 目标父目录 | `RootedFs::link_no_replace`、`CommitKeys` | 保障 | 高 | 可覆盖 winner、发布错误实体或在崩溃后丢目录项 | `EEXIST`、identity swap、fsync 故障；不是 rename |
| MED-U-016 | metadata commit 有 blob 唯一竞争重试，最终 resource upsert 幂等 | `commit_metadata_with_race_retry`、unique index | 保障 | 高 | 并发相同内容会报随机冲突或重复 blob | 双 complete、唯一约束 race、返回 deduplicated |
| MED-U-017 | commit 前再次核对 account enabled、storage path、quota 和对象身份 | `begin_commit`、`ensure_commit_quota` | 保障 | 高 | 上传期间改账户策略后仍可越权提交 | disable、path change、quota shrink |
| MED-U-018 | serve 启动时先 reconcile 未完成 commits、遗留 committed stage 与无引用 blob，之后每 120 秒重复且跳过错过 tick；周期任务不重新 Hash 已完成历史 blob。`reconcile scan` 提供持锁手工入口，无法证明的未完成状态标 unknown 而非伪成功 | `upload_commit::reconcile_all`、`main.rs` | 保障 | 高 | 崩溃后的 stage/final/DB 组合或待回收 blob 会永久卡住；反复扫描历史内容会造成随数据量增长的固定负载 | commit_started/finalizing 各物理组合、committed stage 清理、历史资源版本、orphan blob、周期/手工重试；后台任务无独立 graceful join |
| MED-U-019 | rooted filesystem 拒绝绝对路径、`.`/`..`、symlink、特殊文件和账户根逃逸 | `rooted_fs.rs`、`storage.rs` | 保障 | 高 | 上传或下载可越出 `DATA_DIR` | symlink/rename race、FIFO、嵌套账户路径；当前发布会受控创建 hardlink，数据根须独占写权限 |
| MED-U-020 | account storage paths 全局唯一且不得互相包含，保留 `uploads` 内部目录 | `admin.rs`、doctor | 保障 | 高 | 两个账户可能读写同一物理树 | equal/parent/child/reserved、并发管理变更 |
| MED-U-021 | resource content 按授权 account 打开 blob，流式返回 Content-Length/MIME 和 encoding header | `resource_content` | 核心 | 高 | 无法恢复原始媒体，或可跨账户读取 | own/cross account、missing file、large stream、Content-Length |

## 5. 图库、同步、审计与业务 API

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| MED-L-001 | asset 以 account+device+source_asset_id 唯一，resource 以 asset+source_resource_id 唯一 | Schema unique、upserts | 核心 | 高 | 同一手机媒体重扫会产生重复逻辑对象 | 重扫、不同 device、资源更新 |
| MED-L-002 | `resources.role` 保存 primary/thumbnail 等媒体资源用途，不是身份权限 | `resources.role`、mobile scanners | 核心 | 中 | 客户端无法选择原图与缩略图；误删为“角色清理”会破坏恢复 | primary/thumbnail manifest；不得用于 auth |
| MED-L-003 | timeline 使用 source time+UUID 不透明 cursor，limit 1–250，可筛选 trash/favorite/archived/media kind | `library::timeline` | 核心 | 高 | 大媒体库无法稳定分页或筛选 | 相同时间、下一页、坏 cursor、账户绑定 |
| MED-L-004 | sync 使用 account 自增 sequence，limit 1–1000，返回 next_sequence/has_more | `account_changes`、`sync_changes` | 核心 | 高 | 客户端只能反复全量读取或漏变更 | 空页、多页、并发变更、跨账户 |
| MED-L-005 | 收藏/归档为 asset 布尔状态，PATCH 只改变明确提供字段并记录 change/audit | `update_asset` | 建议保留 | 中 | 备份仍在但无法做基础整理 | 单字段/双字段/空 patch、CSRF不适用数据面 token |
| MED-L-006 | trash/restore 通过明确动作改变 deleted_at；普通列表隐藏 trash | `trash_asset`、`restore_asset` | 建议保留 | 高 | 只能直接永久删或永不删除，误删保护下降 | 重复动作、timeline trashed、change event |
| MED-L-007 | 永久删除只允许 trashed asset；先在同一事务删除 asset/resource、写 change/audit，让无引用 blob 行成为 durable 回收意图，再以 rooted unlink 与 blob-row DELETE 的同一 SQLite 事务收口。物理回收完成返回 204，仍待协调返回 202 | `delete_asset_permanently`、`reconcile_orphan_blobs` | 保障 | 高 | 可绕过回收步骤、误删仍被其他 resource 引用的 blob，或在 unlink 失败后留下数据库无法追踪的对象 | 非 trash、最后/非最后引用、unlink 失败回滚保留行、202、启动/120 秒/`reconcile scan` 重试、commit-after-unlink failure |
| MED-L-008 | 相册按 account+device+source_album_id 同步，支持 replace_members 显式语义 | `sync_album`、Android/iOS scanner | 建议保留 | 高 | 设备相册关系无法保留；错误 replace 会删掉未扫描成员 | full/limited scan、空 album、不同 device |
| MED-L-009 | 标签按账户唯一，支持创建、全量 set、单 asset add/remove | `tags`、`tag_assets` routes | 可选 | 中 | 核心备份不受影响，但人工整理能力下降 | 名称边界、跨账户 ID、重复关系 |
| MED-L-010 | duplicate groups 按账户内 content BLAKE3+size，limit 1–200 | `duplicate_groups` | 可选 | 中 | 无法发现字节相同媒体；仍不会做视觉相似判断 | 同/异账户、同 hash/size、分页上限 |
| MED-L-011 | resource list 最多返回 1000 条；manifest 给出内容路径与 part 规格 | `list_resources`、`ResourceManifest` | 建议保留 | 中 | 简单客户端无法批量检查远端资源 | 上限、排序、账户隔离、metadata |
| MED-L-012 | audit_events 使用 account sequence，记录 device/API key 动作但不保存 token/媒体正文 | `audit.rs`、`api_access::audit_events` | 建议保留 | 中 | 数据面操作难追踪 | actor kind/id、limit 1–500、敏感字段负例 |
| MED-L-013 | device/API key 成功使用最多每 5 分钟更新 last_seen/last_used，避免每请求写放大 | `auth::require_auth` | 保障 | 中 | 每请求写会放大 SQLite 争用；完全不写则失去活跃性 | 5 分钟边界、失败请求不 touch |

## 6. 共享 Rust Client、移动状态与 FFI

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| MED-G-001 | Android/iOS 共用 `crates/client-core` 的发现、准备、上传、重试和完成状态机 | `Client`、`jobs`/`job_parts` | 核心 | 高 | 两端各自实现队列，故障语义快速漂移 | 同 fixture 两平台；Rust state transition tests |
| MED-G-002 | Client config、enqueue、prepared job 都显式携带 product/version/revision/state_epoch | `ClientConfig`、`EnqueueResource`、`PreparedJob` | 保障 | 高 | 非当前宿主或持久 JSON 可被误读 | 每个 identity 字段缺失/错误/额外字段 |
| MED-G-003 | 移动状态固定 `media-backup-mobile-v0.2-r2`、`client-v0.2-r2.sqlite`、`backup-staging-v0.2-r2` | Client constants、Kotlin/Swift contract | 保障 | 高 | 数据库、stage 和 ABI 可能混代 | Rust/Kotlin/Swift/脚本静态合同一致 |
| MED-G-004 | Client 数据库只在文件不存在时创建当前 Schema；现场 DDL fingerprint 精确验证 | `client-core/database.rs` | 保障 | 高 | 非当前队列可能在设备上被自动改写 | empty/wrong metadata/extra table/WAL 零写拒绝 |
| MED-G-005 | Client open 验证所有 persisted `prepared_json` 当前身份后恢复 interrupted 状态 | `Client::open`、`validate_persisted_jobs` | 保障 | 高 | 重启后 preparing/uploading 永久卡住，或坏 JSON 懒到上传时才暴露 | ready/uploading/preparing 有/无 prepared JSON |
| MED-G-006 | `(source_asset_id,source_resource_id)` 唯一；完成且 modified_ms 相同则无需重入队 | `needs_resource`、jobs unique | 核心 | 高 | 每次扫描都重传，或修改后的媒体被错误跳过 | unchanged/changed size/time、非 complete |
| MED-G-007 | enqueue 更新已有 job 时清除 prepared/upload/retry/error，回到 discovered | `Client::enqueue` upsert | 保障 | 高 | 新字节可能沿用旧 parts/upload ID | 修改后 state、part rows/JSON、重启 |
| MED-G-008 | `next_prepared` 会复用 `ready` 的持久 `prepared_json`，再在进程内 Mutex 下领取一个 `discovered` 或到期的 `retry_wait` job；当前没有跨进程 Client 锁 | `Client::next_prepared` | 保障 | 高 | 删除 ready 复用会在每次宿主轮询时重复读取并分块；误称跨进程安全会掩盖重复准备风险 | ready 重取、双线程调用、两个进程边界、空队列、retry time |
| MED-G-009 | `prepare_file` 分块同时计算 part/full BLAKE3，每个 part 写完 `sync_all` | `crates/crypto/src/lib.rs` | 核心 | 高 | 无法续传或证明上传前字节身份 | 空文件、跨 part、I/O failure、Hash fixture |
| MED-G-010 | 宿主导出的临时源在 parts 准备成功并持久化 prepared JSON 后才 best-effort 删除 | `remove_source_after_prepare` | 保障 | 高 | 删除过早会丢唯一临时副本；不删会持续占设备空间 | prepare 失败/成功、删除失败、原始系统媒体永不标记删除 |
| MED-G-011 | mark_upload/part/complete 持久化远端进度；complete 清理本地 prepared parts | Client methods、`job_parts` | 保障 | 高 | 重启后无法续传或 stage 永久累积 | 每状态重启、part cleanup、missing job |
| MED-G-012 | retryable error 使用 2 秒指数退避、最多 1 小时并保留 `prepared_json`；非 retryable 进入 failed | `mark_failed` | 保障 | 中 | 网络故障会热循环，或永久失败与暂时失败无法区分 | retry count 0/10+、到期前后、JSON/part 是否仍在、错误摘要；保留 JSON 不代表当前取队列路径会复用它 |
| MED-G-013 | Client stats 聚合 discovered/ready/uploading/complete/retry_wait/failed | `ClientStats` | 建议保留 | 低 | UI 和诊断无法说明队列停在哪层 | 每状态计数、未知状态由 Schema 阻止 |
| MED-G-014 | 唯一 C ABI 2 使用显式长度、状态码和 owned result；JNI 通过共享边界抛出异常；旧入口已删除 | `mobile-ffi`、生成 Header、NativeBridgeV2/RustClient | 核心 | 高 | 原生宿主无法区分失败或产生跨 allocator 内存错误 | 全导出 C 调用、null/UTF-8、结果释放、JNI 异常及原生验收 |
| MED-G-015 | 宿主先检查 ABI 2，成功业务 JSON 再验证当前 identity；失败不包装成成功 envelope | `sarmg-mobile-ffi`、`MobileContractV02` | 保障 | 高 | Swift/Kotlin 可能接受错误 native library 的返回值 | ABI/version/revision/epoch、明确状态、额外/缺失 keys |
| MED-G-016 | FFI handle registry 管理 Client 生命周期，close 后句柄不可复用 | `mobile-ffi/src/lib.rs` | 保障 | 高 | use-after-close、重复 close 或跨线程状态损坏 | invalid/closed handle、并发调用、资源释放 |

## 7. Android 客户端

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| MED-D-001 | Jetpack Compose 提供 Server/实例授权码、备份范围、自动条件、进度和远端库 UI | `MainActivity.kt` | 建议保留 | 高 | 后台引擎仍在但普通用户无法配置和观察 | 首次空态、保存、旋转、错误/进度 |
| MED-D-002 | EncryptedSharedPreferences 使用 AES256-SIV key 与 AES256-GCM value 保存 endpoint、实例授权码和 token | `SecureConfig.kt` | 保障 | 高 | 普通 preferences/备份中可能泄漏凭据 | 重启、清除、Keystore 失败、数据不可明文搜索 |
| MED-D-003 | endpoint 或实例授权码改变时立即删除现有 bearer token | `SecureConfig` setters | 保障 | 中 | 新实例可能继续带旧设备 Token 请求 | 两字段各自变化/不变、token 清除 |
| MED-D-004 | 支持照片/视频开关、camera-only 或显式相册选择 | `ScanOptions`、`DeviceAlbums`、Compose | 核心 | 中 | 无法控制备份范围或会扫描全部授权库 | 空选择、首次默认、照片/视频组合、相册删除 |
| MED-D-005 | Android 13/14 区分 image/video/selected media 权限，低版本使用 READ_EXTERNAL_STORAGE | `BackupWorker::hasMediaAccess`、Manifest | 保障 | 高 | 部分系统无权扫描或错误宣称拥有全部媒体 | API levels、partial access、拒绝后 UI |
| MED-D-006 | MediaStore 扫描按修改状态入 Client，并导出原始资源和设备缩略图 | `MediaScanner.kt` | 核心 | 高 | Android 无媒体输入或无快速预览资源 | photo/video、损坏 URI、文件名/MIME、thumbnail |
| MED-D-007 | 每批最多扫描 40、上传 60；未完成返回 WorkManager retry | `BackupWorker` constants | 保障 | 中 | 单次 worker 无界运行，易被系统终止 | 超上限、queue 未 drain、下一次继续 |
| MED-D-008 | 全局 Kotlin Mutex 防止同进程两个 BackupWorker 同时驱动同一 Client | `backupMutex` | 保障 | 中 | 队列状态和 stage 可被并发宿主交错 | 自动+手动同时触发、取消后解锁 |
| MED-D-009 | WorkManager 支持立即、周期、Wi-Fi-only、charging-only 与取消 | `BackupScheduler.kt` | 建议保留 | 高 | 只能前台手动备份，可靠性显著下降 | constraints、unique work、配置变更、stop |
| MED-D-010 | 长任务以前台 dataSync notification 发布阶段、项目和进度 | `createForegroundInfo` | 保障 | 中 | 新 Android 会限制后台运行，用户也看不到占用 | notification permission/channel、API 29+ type、取消 |
| MED-D-011 | BackupApi 要求 endpoint 字符串以 `https://` 开头，使用 Bearer 请求 bootstrap/upload/library，并设置 30 秒连接、10 分钟读写 timeout | `BackupApi.kt` | 核心 | 高 | Android 无法与 Server 互操作 | status/body/JSON、token、timeout、TLS failure；尚未做 canonical URL/host allowlist |
| MED-D-012 | 上传只发送 Server 返回的 missing parts，每块成功后持久化 Client checkpoint | `BackupWorker::uploadJob` | 保障 | 高 | 断线后完整重传，或本地误标已上传 | missing subset、part failure、complete failure |
| MED-D-013 | 达到 40 个新资源 scan limit 时使用 `replace_members=false`，否则全量替换已扫描相册成员 | `scan.limitReached`、`syncAlbum` | 保障 | 高 | 分批扫描会错误清空未遍历成员 | 40/41 items、空完整 album；Android 14 selected-media 可见子集当前无法与完整授权区分，存在移除不可见成员的边界 |
| MED-D-014 | RemoteLibrary 分页 timeline、推进 sync cursor、恢复到 MediaStore、读取 thumbnail | `RemoteLibrary.kt` | 核心 | 高 | 用户无法浏览或取回备份 | 多页、cursor、download failure、权限；当前只检查 `plain-v1`/HTTP，不做恢复后 size/BLAKE3 校验；`resolve` 接受绝对 URL 并附带 Bearer，Server 必须只返回同源相对 path |
| MED-D-015 | 收藏、归档、trash/restore、标签和重复组有移动 API/UI 接口 | `BackupApi`、`RemoteLibrary`、Compose | 可选 | 中 | 备份主链保留，整理能力下降 | 每个 mutation、刷新、错误回滚 |
| MED-D-016 | 正式 Android APK 使用全新 `org.sarmg.mediabackup` identity 与唯一 RSA-4096、CA:FALSE 证书；Release Secret 与普通 CI 隔离 | Android Gradle、`release.yml`、`android-signing` Environment | 保障 | 高 | 发布 Debug/未签名/错误 signer APK，或旧应用身份被误当成当前版本 | `assembleRelease`；缺 Secret 失败；alias；唯一 signer；固定 SHA-256；aapt2 package；仅 arm64-v8a；普通 CI 无 Secret |

## 8. iOS/iPadOS 客户端

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| MED-I-001 | SwiftUI 提供配置、相册选择、备份进度、时间线、整理和恢复 UI | `ContentView.swift`、`BackupCoordinator` | 建议保留 | 高 | Rust Client/网络层仍在但没有可用产品入口 | 首次配置、permission、loading/error、空图库 |
| MED-I-002 | 实例授权码/token 使用 Keychain；普通 UserDefaults 只保存 endpoint/偏好/cursor | `KeychainStore.swift`、`BackupCoordinator` | 保障 | 高 | Secret 进入普通偏好/设备备份，或偏好丢失导致体验下降 | 存取/覆盖/删除、重启、Keychain error |
| MED-I-003 | PhotoKit 请求 read-write 权限；authorized 与 limited 都进入当前扫描流程 | `PhotoScanner::requestAccess` | 保障 | 高 | 无法扫描或越过用户授权预期 | authorized/limited/denied/restricted；UI 可见集合不是完整库证明 |
| MED-I-004 | PhotoKit 枚举当前可见相册与成员，按选定集合形成 membership | `PhotoScanner::albums/scan` | 核心 | 高 | 无法保留相册结构或控制范围 | smart/user albums、重复 asset、空 album；limited 结果当前没有“非完整”标志 |
| MED-I-005 | PHAssetResource 原始字节允许从 iCloud 下载到临时源 | `PhotoScanner::export` | 核心 | 高 | iCloud-only 媒体无法备份 | network allowed、下载失败、取消、临时文件 |
| MED-I-006 | iOS 设备生成 512×512 高质量 JPEG thumbnail 并作为独立 `thumbnail` resource | `PhotoScanner::thumbnail` | 建议保留 | 中 | 远端列表仍可取原图但预览代价显著上升 | degraded callback、error、无 image、metadata role |
| MED-I-007 | Background URLSession 上传 part，并用 taskDescription 绑定 job/upload/index | `BackgroundUploader.swift` | 建议保留 | 高 | App 退到后台后传输更易中断，或 callback 无法回到 job | 当前 delegate 活着时 mapping/HTTP/file failure；进程重启事件接管尚未实现，不能声称 relaunch 闭环 |
| MED-I-008 | bootstrap/complete/album 请求使用当前 JSON identity 与 Bearer token | `BackgroundUploader`、`MobileContractV02` | 核心 | 高 | iOS 与 Server/Client 合同分叉 | exact JSON、错误 envelope、token 缺失 |
| MED-I-009 | BackupCoordinator 串行扫描、最多取 24 个 Client job，并把 part 提交给 background session 后同步相册和发布状态 | `runBackup` | 核心 | 高 | UI 与后台组件无法形成完整旅程 | part 仍在飞行时 album sync 只关联 Server 已有 asset；后续 run 才可能补齐，不能称为提交完成后的原子同步 |
| MED-I-010 | RemoteLibrary 提供 timeline/sync、收藏归档、标签、重复数、trash 和恢复到 Photos | `RemoteLibrary.swift` | 核心 | 高 | 备份只能写不能查/恢复 | 多页、cursor、下载、Photos write permission |
| MED-I-011 | 恢复选择 primary resource，把 HTTP 下载临时文件交给 PhotoKit 创建 asset | `restoreToPhotos` | 核心 | 高 | 核心“恢复”目标消失 | 无 primary、下载失败、照片/视频、权限；当前未接入 size/BLAKE3 校验 |
| MED-I-012 | AppDelegate 注册 BGProcessingTask，expiration 会取消 Swift Task | `MediaBackupApp.swift` | 保障 | 中 | 删除后 iOS 不会按系统时机继续调度备份 | 当前未实现 `handleEventsForBackgroundURLSession`；`runBackup` 内部吞掉业务错误且外层仍报告 success，expiration 与正常结束也需防重复 completion |
| MED-I-013 | iOS application、测试 target、BGTask、Keychain、preferences 与 upload session 均位于唯一 `org.sarmg.mediabackup` 当前命名空间 | `project.yml`、`MobileContractV02.swift` | 保障 | 中 | bundle 与持久域分叉，或继续依赖开发占位 identity | xcodegen/plist；bundle IDs；BGTask；Keychain/preferences；源码无开发占位 namespace |

## 9. React/Vite Administrator Web

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| MED-W-001 | Foundation Shell 统一 restore/login/logout、导航、通知与诊断，Session/CSRF 只在内存；产品没有第二套登录状态机 | `createSarmgAdminApplication`、`@sarmg/admin-shell` | 保障 | 高 | 认证竞态或 Secret 持久化 | 共享 Shell 测试、消费者 Chromium/Firefox 验收 |
| MED-W-002 | 统一实例列表、详细信息和日志视图；管理员账号仅由 Foundation Shell 右上角人物图标设置；业务读取失败清除旧数据，安全错误显示 Request ID 和显式重试 | `Application`、`OverviewView`、`UsersView`、`LogsView` | 建议保留 | 中 | 身份域混淆或失败后仍显示过期状态 | 切页、失败/重试、账号设置、无内部错误泄漏 |
| MED-W-003 | 总览聚合 active/total instance、used/pending/quota 和每实例资源数 | `/api/v2/admin/overview`、Overview guard | 建议保留 | 中 | 容量和实例状态只能手工查询 | unlimited quota、large safe integer、空库 |
| MED-W-004 | 新建表单与 Host/Sunshine 一致只填写实例名称，原子创建自动存储、默认配额和客户端授权码，不显示内部账号；停用需确认，授权码可查看、轮换、取消并在终态删除整个空实例；GiB 配额在详情页编辑并保留原始整数字节 | `CreateBackupInstanceDialog`、`BackupUserForm`、`InstanceManager` | 核心 | 高 | 首次配对重新暴露基础设施参数、部分成功留下孤立归属或授权未真正撤销 | name-only create、默认值、edit/disable、实例配对/轮换/删除、精确 quota |
| MED-W-005 | 业务 JSON 在进入组件前校验必需字段与类型，路径只允许 `/api/v2/admin/*` | `web/src/api.ts` | 保障 | 中 | 漂移响应会进入组件，或产品 client 被用于移动路由 | 缺失/错误类型、错误 prefix；当前 guard 容忍响应额外字段 |
| MED-W-006 | Foundation 统一 system/light/dark 主题；产品不读写浏览器存储 | Shell 主题选择器 | 可选 | 低 | 私有外观与平台漂移 | 移动明暗主题 WCAG AA、无横向溢出 |
| MED-W-007 | Foundation tokens/reset/accessibility 提供 focus、reduced motion、forced colors 基线 | CSS imports、`data-sarmg-scope` | 保障 | 中 | 基础行为跨项目漂移 | keyboard、focus、high contrast、CSS digest |
| MED-W-008 | 业务 CSS 仅保留 `.media-*` 布局，导航/表单/弹窗/通知来自共享 UI，字体使用 Maple 同源资产 | `src/styles.css`、Foundation CSS imports | 建议保留 | 中 | 重复平台样式重新分叉 | 禁止私有字体/token、窄屏、长文本、progress |
| MED-W-009 | 精确 Node 26.7.0、React/DOM 19.2.8、TS 5.8.3、Vite 7.3.6 | `.node-version`、package/lock | 开发运维 | 中 | 开发、CI 与发行 bundle 不可复现 | engine、`npm ci`、typecheck、版本断言 |
| MED-W-010 | `build` 强制 check:foundation→typecheck→Foundation Vite 配置，512 KiB 单资产硬预算且禁止 source map；六个 dist 资产含字体与许可 | package scripts、vite config | 开发运维 | 高 | 二进制、字体和发行 manifest 混代 | 构建门禁、实际 dist 浏览器测试 |
| MED-W-011 | 唯一内嵌清单绑定 HTML/JS/CSS/正斜体 WOFF2/OFL，HTTP 与发行校验共用字节；保留 CSP 和 nosniff | `web_assets.rs`、`admin.rs`、`release.rs` | 保障 | 高 | 字体 404、缺少许可证或发布内容漂移 | 实际 HTTP 字节/类型、篡改字体和缺失许可拒绝 |

## 10. SQLite、运行锁、doctor、发布与供应链

| ID | 当前功能/特性与真实行为 | 实现/代码锚点 | 分类 | 复杂度 | 删除后的确定后果 | 最低验证/边界 |
|---|---|---|---|---|---|---|
| MED-R-001 | Server 软件为 0.3.11，数据库 Schema identity 为 media-backup 0.3.0、revision 4、SHA `84f0e8032d8814b8815b0a6a8a499e0e0d7bb44d37932cf78c1e75bd0b5826fe`；管理员和平台 DDL 由 Foundation 组合 | `database.rs`、`schema/generated/current_schema.sql` | 保障 | 高 | 错库或 DDL drift 必须拒绝 | metadata、现场 fingerprint、当前身份精确校验 |
| MED-R-002 | Client 当前 Schema SHA 为 `fb38736bbf8ac69eb694095e62302f73233e39df42cd2d38e3dd1284e2f02558` | `client-core/database.rs` | 保障 | 高 | 手机队列状态不可证明 | Rust/Kotlin/Swift epoch 与 Schema identity |
| MED-R-003 | 两个数据库都先复制 main/WAL/journal 私有 generation，再验证 source 未变化 | 两个 `database.rs` | 保障 | 高 | 启动验证可能读取跨时刻混合状态或写源库 | WAL、并发变化、symlink、cleanup |
| MED-R-004 | Server open 使用 WAL、foreign keys、busy timeout，并在业务前做 integrity/FK | `database.rs`、doctor | 保障 | 高 | 并发/损坏行为变得不可预测 | PRAGMA、busy 5s、corruption、FK violation |
| MED-R-005 | runtime lock 同时绑定数据库与 DATA_DIR，防止两个 Server 管同一状态 | `runtime_lock.rs` | 保障 | 高 | 双实例可同时提交、清理和改账户路径 | 同 DB/不同 data、同 data/不同 DB、symlink/hardlink |
| MED-R-006 | doctor 校验 Schema、integrity/FK、rollback write probe、storage write cleanup，为数据树普通文件计算 Hash 后核对 DB blob、durable part 与 active commit，并拒绝尚未收口的无引用 blob 行 | `doctor.rs` | 开发运维 | 高 | 上线与故障只能靠零散检查，损坏 blob 或待回收意图可能长期潜伏 | missing/mutated、unknown commit、orphan blob、read-only；先运行 `reconcile scan` 再复查 |
| MED-R-007 | `/healthz` 为最小存活检查；`/readyz` 检查 DB/storage；详细诊断要求管理员 | Foundation Runtime | 开发运维 | 中 | 负载均衡需区分可响应与可写服务 | 匿名无内部信息、诊断认证、任务监督 |
| MED-R-008 | Prometheus 文本只暴露聚合 user/device/resource/bytes/upload 指标 | `metrics.rs` | 开发运维 | 中 | 容量不可监控；若加 labels 不慎会泄漏用户名/路径 | content type、token、无高基数/Secret |
| MED-R-009 | release identity 绑定 source revision、target、API、encoding、Schema、FFI header 与 Web bytes | `release.rs` | 保障 | 高 | 二进制、移动 ABI 和 Web 可混代 | identity JSON/contract hash、单字段篡改 |
| MED-R-010 | 全树 manifest 精确约束文件、mode、size、SHA，拒绝额外/缺失/链接 | `release.rs`、manifest writer | 保障 | 高 | 安装树内容无法证明 | missing/extra/tamper/symlink/hardlink/mode |
| MED-R-011 | 正式 binary 只能从规范 release root 用 `serve-release` 启动；普通 serve 仅 unversioned 开发 build | `verify_runtime`、`ensure_unbound_development_serve` | 保障 | 高 | source-bound binary 可绕过发行闭包 | physical executable path、wrong root、release build serve |
| MED-R-012 | `build-server-release.sh` 只在 Linux AMD64 接受 64-bit little-endian x86_64 ELF | release script | 开发运维 | 中 | 文件名 target 与真实 ELF 可不一致 | ELF magic/class/endian/machine、wrong host |
| MED-R-013 | 发行包包含 binary、配置样例、`deploy/media-backup.service` 映射出的 systemd、脚本、Web、FFI header 和必要文档 | build script | 开发运维 | 高 | 操作者拿到不完整或跨代部署单元 | expected exact layout、真实 verify-release |
| MED-R-014 | systemd 使用 `isarmg-media`、flat `/etc/isarmg/media-backup.env`、ConditionArchitecture 和 sandbox | `deploy/media-backup.service` | 保障 | 高 | 错服务账号、配置路径或权限扩大主机攻击面 | `systemd-analyze verify`、实际 start、write paths |
| MED-R-015 | 安装 no-clobber 固定 `/opt/isarmg/media-backup/releases/0.3.11`，环境 0600 | `setup-wsl.sh`、deployment tests | 保障 | 高 | 同版本覆盖会让运行内容不可追溯，Secret 权限过宽 | 首装/二次安装、concurrent、mode/owner |
| MED-R-016 | CI 分别覆盖 Rust/Server release、Android、iOS 和移动静态合同 | `.github/workflows`、contract scripts | 开发运维 | 高 | 任一平台可在 wire/FFI 漂移时独立发布 | clean checkout jobs、平台矩阵、lock mode |
| MED-R-017 | Rust 固定 1.98.0；Web Node/toolchain 与 Cargo/npm locks 均固定 | toolchain/version/lock files | 开发运维 | 中 | 解析随时间变化，制品难复现 | `--locked`、`npm ci`、version output |
| MED-R-018 | 中文 README、学习、流程、功能取舍和运维文档是发行/维护闭包 | `README.md`、`docs/` | 开发运维 | 低 | 跨五种语言/平台的知识只能口头传递 | 链接、命令、代码锚点和 schema hash 抽查 |

## 11. 明确边界与产品取舍

| ID | 当前决定 | 实现/边界锚点 | 分类 | 复杂度 | 若改变会发生什么 | 实施前最低证据 |
|---|---|---|---|---|---|---|
| MED-X-001 | 不提供 Server ARM/musl/Windows/macOS；移动客户端仍支持各自正式 ABI | compile/release/runtime gates | 核心 | 高 | Server 扩平台需重做文件系统、systemd、脚本和发行证明 | 新平台完整 CI、真实部署、等价安全测试 |
| MED-X-002 | 不提供桌面同步客户端或双向文件夹镜像 | 无 desktop client/conflict Schema | 核心 | 高 | 会引入文件名、冲突、删除传播和任意文件类型模型 | 独立产品 RFC、冲突状态机、多平台矩阵 |
| MED-X-003 | 不提供端到端加密；Server 管理员可读取媒体 | `plain-v1` | 核心 | 高 | E2EE 会改变 key recovery、缩略图、去重、doctor 和恢复 | 双端密钥生命周期、灾备、不可恢复风险设计 |
| MED-X-004 | 不提供服务端视频转码、RAW 派生或媒体解码 | 无 worker/codec deps | 核心 | 高 | 引入不可信解析攻击面、作业队列和大量派生状态 | sandbox、资源预算、job recovery、供应链 |
| MED-X-005 | 不提供人脸识别、语义搜索、地图或视觉相似去重 | 无 model/index Schema | 可选 | 高 | 增加生物特征隐私、模型版本和索引删除责任 | 明示同意、模型 SBOM、删除/重建、算力预算 |
| MED-X-006 | 不提供公开分享链接或跨账户协作 | 所有资源 route 绑定 account auth | 核心 | 高 | 需要公开 token、撤销、滥用控制和新的授权关系 | threat model、限流、审计、有效期 |
| MED-X-007 | 不跨账户去重；blob unique 以 account 为边界 | `blobs_content_unique_idx` | 保障 | 高 | 跨账户内容存在性侧信道和计费归属问题 | 隐私证明、配额/删除语义、加密策略 |
| MED-X-008 | 不自动清空 trash；永久删除必须显式调用 | library routes | 建议保留 | 高 | 自动策略配置错误会不可逆丢失备份 | retention、dry-run、审计、恢复窗口测试 |
| MED-X-009 | 不在产品运行时迁移、双读、alias route 或扫描其他代 state | strict router/schema/epoch | 保障 | 高 | 维护成本随代数增长，错误输入可能触发写入 | 转换进入 `sarmg-upgrade`；产品保留唯一格式 |
| MED-X-010 | 不把 `accounts` 或 `resources.role` 合并进 Administrator 角色 | 分离 Schema/route/auth | 核心 | 高 | 数据归属、媒体用途和控制权限会混成不可审计模型 | 三域术语、逐 route threat model、升级转换 |
| MED-X-011 | 管理 Web 不是完整媒体图库；时间线/恢复/整理主要由移动端承担 | Web routes/components | 可选 | 高 | 完整 Web 图库需增加安全下载、虚拟列表、预览与恢复 UX | 大库性能、隐私/CSP、浏览器媒体测试 |
| MED-X-012 | 原始媒体、缩略图、SQLite 和 external TLS/Secret 必须作为组合灾备对象；产品不执行备份 | 运维边界、`sarmg-upgrade` | 保障 | 高 | 只复制 DB 或 data 会得到不可恢复组合 | 停机锁、sidecar-aware snapshot、隔离恢复演练 |
| MED-X-013 | Client 当前**没有**在到期 `retry_wait` 中复用已有 `prepared_json`；它会重新进入 `prepare_file`。若 `remove_source_after_prepare=true` 已删除宿主导出的临时源，第一次上传失败后可能永久卡在“源文件不存在”重试 | `Client::next_prepared`、`mark_failed`、`prepare_job` | 保障 | 高 | 修复需要把“已有 prepared parts 的上传重试”与“尚未准备成功的源文件重试”分开；忽略该边界会把可重试网络错误变成不可恢复队列 | 精确测试：准备并删源→mark upload/failed→推进 backoff→下一次必须复用同一 JSON/part 且不读源；当前测试与实现尚未满足 |
| MED-X-014 | Client 当前**没有**清理 `prepare_file` 中途失败或 `preparing` 无 JSON 的进程恢复残留；`prepare_file` 会复用既有 job 目录并覆盖同名 part，尾部旧 part 可能留下 | `crates/crypto::prepare_file`、`Client::open/next_prepared/mark_complete` | 保障 | 高 | 修复必须只操作 `backup-staging-v0.2-r2/prepared/<规范 job UUID>/`，不能宽泛删除 `prepared/` 或按未验证 DB 文本拼接路径；忽略会长期占用设备空间 | 精确测试：准备失败残留、进程恢复残留、短文件覆盖长文件、非法 job ID、相邻 job 不受影响；当前实现尚未满足 |

## 12. 关键取舍说明

### 12.1 为什么 Server 只支持 AMD64，而移动端不是

Server 的 target gate、x86_64 ELF、systemd 和 Linux 文件系统安全形成一套部署证明；Android/iOS Rust
库只是设备内 Client，需要各自 ARM/模拟器 ABI。把两者一概写成“全项目 AMD64”会直接破坏真机客户端，
因此平台矩阵必须按 Server 与 client 分栏维护。

### 12.2 为什么有 Administrator、备份账户和资源 role

Administrator 管理部署；备份账户拥有媒体命名空间、配额和设备；resource role 说明一个 asset 的原始
资源或缩略图。三者作用于不同对象、使用不同凭据，也有不同删除后果。统一“只保留管理员”是指取消
控制面 RBAC，不是删除数据面账户或 `primary`/`thumbnail` 语义。

### 12.3 为什么按块上传但最终保存原始字节

分块服务于移动网络续传和有界请求，并不改变存储格式。Server 只有在全部 part 和完整 BLAKE3 都成立后
才发布一个 `plain-v1` blob。这样恢复简单，但服务器管理员能读取内容，必须用 TLS、磁盘加密、最小权限
和加密备份补足机密性。

### 12.4 为什么不在运行时做升级

Server 与 Client 在打开状态前先验证唯一当前 Schema 和身份。发现非当前状态即停止，避免一边服务一边
转换导致半代数据。未来转换、备份和恢复只由 `sarmg-upgrade` 在停机排他边界执行。

## 13. 功能删除检查表

1. 在评审中引用本清单 ID，明确接受的用户后果与数据后果。
2. 删除所有生产者和消费者：Server、Client、FFI、Android、iOS、Web，不保留隐藏入口。
3. 涉及持久状态时生成新的完整当前 Schema；产品代码不添加 migration。
4. 同步 API DTO、mobile epoch、FFI header、release identity 和静态合同。
5. 删除配置、依赖、权限、systemd/脚本和发行 manifest 条目。
6. 加入当前合同负例，证明退役入口和字段不再可用。
7. 同步 README、学习指南、流程树、本清单、运维文档与 `sarmg-upgrade` 资源合同。

只有闭包全部完成，功能才算真正删除；配置成 false、隐藏按钮或停止某个平台测试都不能减少其维护责任。

## 图库扩展实施补充

本次新增全库类型/日期/设备筛选、UUID 快照水位、资产/资源/标签批量 SQL、1600 像素派生预览，
以及鉴权 Range/HEAD/ETag 内容读取；具体协议与验证见 [图库 API 扩展](https://github.com/isarmg/media-backup-server/blob/v0.3.11/docs/gallery-api.md)。
移动实现以 Client 0.4.0 的实施记录为准，本文原移动端清单描述的是改造前快照。
