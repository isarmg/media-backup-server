# 工程组织与责任边界

此页描述 1.0.0 发行候选。正式发行身份由精准 Source 的 CI 与 Release 资产证明。

## 目录约定

根 `Cargo.toml` 是多包 workspace；所有依赖版本与 Rust lint 在根声明。`crates/server/` 保存 Server 源码、构建入口和业务测试；`crates/protocol/` 保存 `xszs-protocol` 唯一业务协议源码。内部包继承 workspace 依赖和 `unsafe_code = "forbid"`，不各自维护公共版本。

`web/` 保存管理界面；`config/` 保存示例；`schema/` 保存当前 DDL 和生成定义；`scripts/` 保存构建、检查与打包；`deploy/` 保存运行环境与服务示例；`docs/` 保存设计、任务说明和发行记录。仅根目录管理 `Cargo.lock`。同类目录保持相同职责，不按产品名或语言重新命名；目录迁移必须同时校验包含路径、CI 与实际归档来源。

## 模块与定义来源

管理 Web 中 `OverviewView.tsx` 维护有界实例页及服务器全局统计，`UserDetailsView.tsx` 维护单实例编辑/授权，`LogsView.tsx` 维护日期、实例作用域、日志游标与请求预算；两种分页复用 `PageNavigation.tsx`。`main.tsx` 只组合导航与按 ID 的详情读取，不包含分页实现。

领域按 config / lifecycle / administration / media / upload / storage 划分。`routes.rs` 与 `admin.rs` 执行入口授权，`library.rs` 管理媒体语义，`upload_commit.rs` 管理持久提交与恢复，`storage.rs` / `rooted_fs.rs` 管理实际文件边界，`database.rs` 管理当前 Schema。 `main.rs` 组合配置、初始化、运行与停止；入口不重新实现 xcss 机制。

软件身份来自 Cargo，发行配置须与其一致；业务协议来自 `crates/protocol/`，当前 DDL 来自 `schema/`。当前 Schema revision/指纹为 1、`0e37f8a3992b1904215d5f4c9ac428448752718506611f818482d890322d300d`，此次保持不变。Rust 与 Web xcss 使用已发布的完整 Git revision、精确版本或具备完整性摘要的发行包，不依赖同级源码作为正式构建输入。候选 xcss 联调只能作为明确记录的临时覆盖，不能伪装成已发布依赖。

## 运行与安全边界

正式目标保持 `x86_64-unknown-linux-gnu`；其他目标的 Server 构建明确拒绝。业务代码及必要管理 Web 嵌入单个可执行程序，TLS 入口、权限和环境依赖见部署文档。

公共配置、CLI、日志、认证、运行生命周期、SQLite 和安全状态文件来自 xcss；产品维护实例归属、权限语义、业务持久条件及资源上限。`init` 显式创建空的私有状态；`run` 严格验证现有状态；`config validate` 使用只读验证快照；运行和独占维护使用共同目录锁。未满足当前契约时不得重建或猜测修复。

## 验证与支持限制

发行必须使用锁定工具链与 Cargo/npm 锁、明确源码 revision，并运行格式、静态检查、相关风险测试及最终归档验收。当前开发环境是 macOS ARM64；协议测试和管理 Web 测试不能代替 Linux Server 运行与发行物验收。测试夹具不能代替真实主机或设备。实际执行结果见本次候选发行说明；正式完成状态以精准 Source 的远端 CI、Release 和最终制品验收分别证明。

Rust 固定 [官方当前稳定版 1.99.0](https://blog.rust-lang.org/releases/)，不为制造升级而切换到预发行工具链。依赖最低版本与锁均更新到当前合适稳定线；SQLx 0.9.0、Tokio 1.53.2、Axum 0.8.9 保持已有当前主线，重新解析的传递依赖记录在根 Cargo.lock 中。unsafe 结论见 [审查记录](unsafe-audit.md)。

## 统一规范适用验收

| 条款 | 实现与证据 | 验证边界 |
|---|---|---|
| 二～五：职责、方向、目录与权威来源 | 根 workspace 统一依赖/lint；`crates/server` 与 `crates/protocol` 单向依赖；xcss 使用完整 Git revision；配置、DDL、protocol 各有唯一来源 | 保持既有规范目录，没有随意重命名或迁移 |
| 六～九：错误、当前契约、授权、配置/CLI | xcss 类型化错误、request ID、Session/CSRF；业务入口验证归属/许可；`init` 与 `run` 分离，config 子命令共享解析与结构校验 | 既有 CLI/权限/拒绝坏输入测试在 Linux CI 运行；跨编译不能代替运行 |
| 十～十二：交付、生命周期、状态/文件 | `build.rs` 从受控 Web manifest 嵌入资源；打包脚本验证单二进制及 Source/资产身份；runtime 锁、当前 schema/数据、密钥和实际文件身份检查 | release 工具离线测试已执行；最终 Linux 安装、systemd 和实际恢复需发行验收 |
| 十三～十五：任务、上限、日志/实例 | 业务操作区分接收、提交与观察；当前任务/上传恢复不猜测历史；分页/响应预算、队列或上传额度控制资源；xcss 文件日志与产品实例事件共同关联 request ID | 本次相关协议/分页/能力回归见发行说明；真实网络和设备不能由夹具代替 |
| 十六：界面一致性 | 共享 Shell/UI、内存会话、正确空/错误/等待状态；能力和权限最终由 Server 执行；嵌入 UI 与 Server 同批发行 | Chromium/Firefox 行为、布局与 WCAG 自动检查；不是实际设备验证 |
| 十七：摄像头适配 | 本产品没有摄像头适配职责 | 不适用，不创建产品无关分支 |
| 十九～二十五：依赖、CI、测试、文档与交付 | Rust 1.99.0 当前稳定线、真实 Cargo/npm 锁、不可变来源和全目标 CI；`docs/unsafe-audit.md` 记录产品边界；发行工具作为独立职责，业务 Server 不包含其实现或专用集成 | 本地协议、Web、release-tool 检查与 Linux 静态编译分别记录；以精准提交 CI 和最终资产决定是否可发布 |
