# 日常运维

安装和初始配置见[发行包手册](server-release-readme.md)。服务以专用 `xszs` 用户运行；配置在 `/etc/isarmg/xszs.env`。

## 查看、启动和停止服务

```sh
sudo systemctl status xszs.service --no-pager --full
sudo journalctl -u xszs.service --since today --no-pager
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/readyz
```

两个健康请求均应成功；`/healthz` 表示存活，`/readyz` 检查数据库和存储。再从公开 HTTPS 地址检查 `/admin` 和一次测试上传。

启动已配置部署时使用包内脚本，它会复核发行、unit 和配置：

```sh
sudo /opt/isarmg/xszs/releases/1.0.0/scripts/start-server-wsl.sh
```

停止使用 `sudo systemctl stop xszs.service`，确认进程退出后再执行维护。`run-server-wsl.sh` 会启动服务并跟随日志，Ctrl+C 仅结束查看。

## 容量与日志

监控数据库和媒体文件系统的空间、inode、I/O 错误，以及就绪状态、重启次数、上传错误和证书到期时间。
`METRICS_TOKEN` 非空时可用独立 Bearer Token 读取 `/metrics`；它只提供聚合数量和字节统计。令牌留在受保护监控配置中。

日志查询日期与管理页面一致，按服务器本地时区。分享问题时提供时间范围、实例 ID、请求 ID、错误码和版本；媒体、配置和凭据保留在受控主机。

## 离线诊断和协调

`doctor` 和 `reconcile scan` 使用排他维护锁。先停服并确认已退出，再以原服务用户、原环境执行：

```sh
sudo systemctl stop xszs.service
sudo systemctl status xszs.service --no-pager
sudo systemd-run --wait --collect -p User=xszs -p Group=xszs \
  -p EnvironmentFile=/etc/isarmg/xszs.env \
  /opt/isarmg/xszs/releases/1.0.0/bin/xszs doctor
```

停服后的 `systemctl status` 返回非零是正常现象，应看到 inactive 且无服务进程。Doctor 检查当前结构、SQLite、对象 Hash、上传状态与存储探针。
若报告待处理提交或待回收对象，核对诊断后执行：

```sh
sudo systemd-run --wait --collect -p User=xszs -p Group=xszs \
  -p EnvironmentFile=/etc/isarmg/xszs.env \
  /opt/isarmg/xszs/releases/1.0.0/bin/xszs reconcile scan
```

该命令会完成待处理上传、回收已记账的无引用 blob 并清理相应暂存项。完成后重新运行 Doctor，再用启动脚本恢复服务。
诊断有目录、读取量和时限预算，超限会明确失败；完整参数与协调语义见[运行参考](runtime-reference.md#存储协调与诊断预算)。

## 故障排查

| 症状 | 检查 | 预期结果与下一步 |
|---|---|---|
| 安装拒绝已有目录或 unit | 目标来源及当前部署 | 该安装器只做全新安装，保留已有部署后选择正确任务 |
| 启动要求审阅初始秘密 | `/etc/isarmg/xszs.env` | 确认三种秘密用途及值，删除已确认标记，保持 root/0600 |
| 服务立即退出 | Journal 首条错误、发行身份、路径和权限 | 修正对应输入，保持当前数据库结构与密钥匹配 |
| 存活正常、就绪 503 | 存储挂载、容量、权限、SQLite 错误 | 探针恢复成功后再恢复业务入口 |
| HTTPS 语义错误 | 真实代理对端、TLS、`TRUSTED_PROXY_CIDRS` | 仅信任实际直连代理，正确传递 HTTPS 信息 |
| 登录失败 | 管理员 username、Cookie、时间与限流 | 使用管理员 username 和密码；手机授权码用于配对 |
| 上传失败 | 手机权限/队列、代理 body 限制、分块大小、并发和空间 | 按失败位置修正，再由客户端继续任务 |
| 数据结构或对象 Hash 失败 | Doctor 输出和精确程序身份 | 保留原状态，停止写入并调查，避免直接改 metadata 或删文件 |

## 安全事件

先在代理层隔离受影响入口，保留脱敏日志、发行身份和时间线，再按影响范围撤销或轮换管理员、设备、指标及主机凭据。密钥变更需考虑已有授权码密文；生产数据只在受控环境中调查。
