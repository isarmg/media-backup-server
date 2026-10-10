# 当前服务命令

所有命令使用同一份当前 JSON 配置：`xszs --config /absolute/config.json --data-dir /absolute/data ...`。配置来源优先级为命令行、明确映射的环境变量、文件、默认值。未知字段、重复字段、错误类型和坏的低优先级配置都会被拒绝；诊断不会打印秘密值。`config validate --json` 中 `sources` 是字段的来源，`schema_identity` 是校验过的当前结构身份，`state_paths` 列出全部私有持久状态的绝对路径。

```sh
xszs --help
xszs --version
xszs init --config /absolute/config.json --data-dir /absolute/data
xszs config validate --config /absolute/config.json --data-dir /absolute/data --json
xszs run --config /absolute/config.json --data-dir /absolute/data
xszs status --config /absolute/config.json --data-dir /absolute/data --json
```

`init` 是创建数据库和首个管理员的唯一部署入口，只接受尚未初始化的私有空目录。配置和管理员凭据必须先有效；已有数据不会被覆盖。数据目录应属于服务账号、权限为 `0700`，私有文件权限为 `0600`。正式制品的运行还需指定 `run --release-root /absolute/release`。

`run` 只接受完整的当前数据；缺失数据库、管理员、结构漂移或身份不符均失败，不隐式初始化或重置账号。运行及写入维护命令使用同一个数据目录维护锁；整个运行期间持有实例锁。`.state-maintenance-pending.json` 存在时拒绝运行、初始化或写入维护；只读检查仍可运行。

`config validate` 读取私有 SQLite 验证快照，原库、WAL、SHM 和业务文件保持不变。`status` 查询当前监听地址的 `/readyz`，核对服务身份和真实业务就绪；端口占用、其他服务、连接失败或未就绪均返回非零退出码。`--json` 输出单个机器记录；失败返回稳定 `code/message/details` 错误记录。帮助和版本查询不要求初始化。

共享配置、命令、快照与日志均固定到同一 xcss Git 完整提交和精确版本；Web 包使用封存制品的真实 SHA-512 完整性。当前是发行候选；正式产品发布以精准 Source 的 CI 和 Release manifest 为准。正式发行必须从这些精确输入独立构建并验证最终制品。

媒体数据库使用明确配置的绝对私有路径；`state_paths` 另外列出数据库父目录，覆盖数据库和 WAL/SHM。上传和媒体目录在绝对 `data_dir` 内。校验同时检查管理员、存储目录和已保存授权码的解密与摘要；数据结构不符时只读失败。

`init` 同时创建 `data_dir/logs` 私有目录；`run` 验证该目录后写入共享 JSON 日志。默认单文件上限 8 MiB、保留 4 个归档，总上限 40 MiB。配置和状态命令不打开运行日志文件。

正式运行可使用安装目录下唯一受控的 `current` 链接：安装目录与 releases 目录必须是同一所有者的实体 `0755` 目录，链接只能指向该安装目录中当前软件版本的绝对实体路径；完整发行树验证仍然执行。
