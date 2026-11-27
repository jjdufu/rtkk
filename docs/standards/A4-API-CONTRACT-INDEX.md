# A4 · Daemon HTTP / SSE 路由契约索引表

本文档维护 `rtkk serve` 全部 `/api/v1/*` 路由的权威注册表。[scripts/check_routes.py](../../scripts/check_routes.py) 在 G1 门禁阶段对「第 1 节 · 已挂载路由表」与 `src/daemon/routes/**/*.rs` 源码实际注册路由执行**双向集合等价校验**：任何源码已挂载但未登记、或已登记但源码缺失的路由均直接阻断门禁。

---

## 1. 已挂载路由表（Mounted Routes · 共 34 条 · `check_routes.py` 双向强校验）

<!-- MOUNTED_ROUTES_START -->
| 方法 | 路由路径 | 鉴权级别 | 互斥级别 | 处理模块 | 所属步骤 | 接口摘要 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `GET` | `/api/v1/health` | `public` | `readonly` | `routes/health.rs` | Step 02 | 探活端点，返回 `{status: "ok", version, uptime_secs}` |
| `POST` | `/api/v1/auth/login` | `public` | `readonly` | `routes/auth.rs` | Step 02 | 校验管理令牌并签发 `HttpOnly; SameSite=Strict` 会话 Cookie |
| `POST` | `/api/v1/auth/logout` | `authenticated` | `readonly` | `routes/auth.rs` | Step 02 | 销毁当前内存会话并清除客户端 Cookie |
| `GET` | `/api/v1/auth/me` | `authenticated` | `readonly` | `routes/auth.rs` | Step 02 | 返回当前认证状态、工作区绝对路径与服务端版本 |
| `GET` | `/api/v1/system/overview` | `authenticated` | `readonly` | `routes/health.rs` | Step 02 | 返回工作区子目录就绪度、离线包统计、最近任务与活跃锁状态 |
| `GET` | `/api/v1/jobs` | `authenticated` | `readonly` | `routes/jobs.rs` | Step 02 | 分页查询运行中与历史异步任务列表 `{items, next_cursor}` |
| `GET` | `/api/v1/jobs/{id}` | `authenticated` | `readonly` | `routes/jobs.rs` | Step 02 | 获取单个任务元数据、起止时间戳、终态与节点汇总 |
| `POST` | `/api/v1/jobs/{id}/cancel` | `authenticated` | `readonly` | `routes/jobs.rs` | Step 02 | 触发任务级取消句柄协作中止执行 |
| `GET` | `/api/v1/jobs/{id}/events` | `authenticated` | `readonly` | `routes/jobs.rs` | Step 02 | SSE 事件流：先回放落盘 JSONL 历史帧，再无缝尾随实时事件 |
| `GET` | `/api/v1/inventory` | `authenticated` | `readonly` | `routes/inventory.rs` | Step 04 | 读取 `********` 脱敏后的结构化配置、拓扑组与脱敏 YAML 原文 |
| `PUT` | `/api/v1/inventory` | `authenticated` | `mutating` | `routes/inventory.rs` | Step 04 | 校验拓扑与参数，无损保留未改密文、留存 `.bak` 并原子写回 |
| `POST` | `/api/v1/inventory/validate` | `authenticated` | `readonly` | `routes/inventory.rs` | Step 04 | 只读校验候选 Inventory 并返回动态派生集群变量（不写盘） |
| `POST` | `/api/v1/inventory/probe` | `authenticated` | `readonly` | `routes/inventory.rs` | Step 04 | 并发只读探测指定主机 SSH/`sudo`/`su`、Debian 13 版本、架构与网卡 |
| `POST` | `/api/v1/templates/render` | `authenticated` | `readonly` | `routes/templates.rs` | Step 04 | 针对选定节点实时渲染 6 大 `.gtmpl` 模板及 `images.yaml` 并校验语法 |
| `GET` | `/api/v1/artifacts/status` | `authenticated` | `readonly` | `routes/artifacts.rs` | Step 05 | 盘点二进制、归档、Debian 13 依赖包及镜像 `.tar` 的落盘与校验态 |
| `POST` | `/api/v1/artifacts/download` | `authenticated` | `mutating` | `routes/artifacts.rs` | Step 05 | 异步触发 `download.yaml` 指定范围预下载任务，返回 `{job_id}` |
| `POST` | `/api/v1/registry/ping` | `authenticated` | `readonly` | `routes/artifacts.rs` | Step 05 | 在线探测私库 HTTPS/HTTP 协议、Basic/Bearer 令牌交换与 Harbor 可达性 |
| `POST` | `/api/v1/registry/pull` | `authenticated` | `mutating` | `routes/artifacts.rs` | Step 05 | 异步批量或按子集拉取上游 OCI 镜像至本地 `.tar`，返回 `{job_id}` |
| `POST` | `/api/v1/registry/push` | `authenticated` | `mutating` | `routes/artifacts.rs` | Step 05 | 异步校验本地 `.tar`、自动建 Harbor 项目并推送私库，返回 `{job_id}` |
| `GET` | `/api/v1/harbor/projects` | `authenticated` | `readonly` | `routes/harbor.rs` | Step 05 | 列出 Harbor 项目并与当前透明映射表及兜底项目执行对账 |
| `POST` | `/api/v1/harbor/projects/sync` | `authenticated` | `mutating` | `routes/harbor.rs` | Step 05 | 幂等创建所有缺失的 Harbor 透明映射源站项目与兜底项目 |
| `GET` | `/api/v1/harbor/projects/{project}/repositories` | `authenticated` | `readonly` | `routes/harbor.rs` | Step 05 | 查询指定 Harbor 项目下的仓库列表（支持关键词过滤） |
| `GET` | `/api/v1/harbor/projects/{project}/artifacts` | `authenticated` | `readonly` | `routes/harbor.rs` | Step 05 | 查询指定 Harbor 项目与仓库下的 Artifact / Tag 元数据列表 |
| `DELETE` | `/api/v1/harbor/projects/{project}/artifacts` | `authenticated` | `mutating` | `routes/harbor.rs` | Step 05 | 删除指定 Harbor 仓库下的目标 Artifact / Tag |
| `POST` | `/api/v1/harbor/mirror/plan` | `authenticated` | `readonly` | `routes/harbor.rs` | Step 05 | 批量解析原始镜像地址，预览透明目标 Harbor 路径并检测是否已存在 |
| `POST` | `/api/v1/harbor/mirror/import` | `authenticated` | `mutating` | `routes/harbor.rs` | Step 05 | 异步批量拉取上游镜像并导入 Harbor 透明映射目录，返回 `{job_id}` |
| `POST` | `/api/v1/operations/prep-hosts` | `authenticated` | `mutating` | `routes/operations.rs` | Step 06 | 异步执行集群 `prep hosts` 主机名采集与 `/etc/hosts` 同步 |
| `POST` | `/api/v1/operations/run` | `authenticated` | `dynamic` | `routes/operations.rs` | Step 06 | 异步执行内置 Playbook（仅 `tags: ["check"]` 为只读，其余加排他写锁） |
| `GET` | `/api/v1/playbooks/{name}/outline` | `authenticated` | `readonly` | `routes/operations.rs` | Step 06 | 解析并返回 `deploy`/`cleanup`/`download` 的 Play、Task 与可选 Tags 树 |
| `GET` | `/api/v1/pki/certificates` | `authenticated` | `readonly` | `routes/pki.rs` | Step 06 | 扫描 `pki/` 下公开证书 X.509 元数据（CN、SAN、到期日，绝不含私钥） |
| `POST` | `/api/v1/pki/generate` | `authenticated` | `mutating` | `routes/pki.rs` | Step 06 | 在线签发 99y/1y 根 CA 或含 IP/DNS SAN 的叶证书落盘至 `pki/custom/` |
| `POST` | `/api/v1/pki/verify` | `authenticated` | `readonly` | `routes/pki.rs` | Step 06 | 校验工作区路径内证书、私钥配对关系与根 CA 签名链 |
| `GET` | `/api/v1/backups` | `authenticated` | `readonly` | `routes/pki.rs` | Step 06 | 列出 `rtkk_bak/` 下已回收的节点状态、`admin.conf` 与 PKI 备份文件清单 |
| `GET` | `/api/v1/backups/archive` | `authenticated` | `readonly` | `routes/pki.rs` | Step 06 | 将 `rtkk_bak/` 流式压缩为 `.tar.gz` 下载并写入审计日志 |
<!-- MOUNTED_ROUTES_END -->

---

## 2. Stage B 待挂载路由预登记全表（Planned Routes · 剩余 0 条）

<!-- PLANNED_ROUTES_START -->
| 方法 | 路由路径 | 鉴权级别 | 互斥级别 | 目标处理模块 | 交付步骤 | 接口摘要 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
<!-- PLANNED_ROUTES_END -->
