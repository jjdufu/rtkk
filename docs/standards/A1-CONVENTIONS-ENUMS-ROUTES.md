# A1 · 模块词表、CLI 子命令、前端路由、公共枚举与环境变量规范

本文档作为全仓命名、枚举序列化字面量、前端路由与环境变量的唯一标准词典，供 Rust 核心引擎、Daemon 路由与 Park UI 前端强类型层同源对齐。

---

## 1. Rust 顶层模块词表（`src/`）

| 模块路径 | 状态 | 核心职责边界 |
| :--- | :--- | :--- |
| `src/main.rs` | 已交付（Phase 0） | 二进制唯一入口，驱动 Tokio 运行时与日志优雅刷新 |
| `src/lib.rs` | 已交付（Phase 0） | 库根导出与 Workspace 级零豁免 Lint 契约 |
| `src/cli.rs` | 已交付（Phase 0） | `clap` 子命令解析、Inventory 加载派生与命令分发 |
| `src/error.rs` | 已交付（Phase 0） | 统一结构化错误枚举 `RtkkError` |
| `src/connector/` | 已交付（Phase 0） | `LocalConnector`、`SshConnector`、`PrivilegeConfig`（`sudo`/`su`）与 `HostKeyPolicy` |
| `src/inventory/` | 已交付（Phase 0） | 扁平/层级双模 Inventory 解析、DAG 环路检测、确定性排序与 `localhost` 隔离 |
| `src/playbook/` | 已交付（Phase 0） | Playbook 编译、Roles 展开、并发批次引擎与集群动态变量注入 |
| `src/tasks/` | 已交付（Phase 0） | 10 类内置原子任务模块执行器（含 `ensure_harbor_projects`）与幂等结果聚合 |
| `src/template/` | 已交付（Phase 0） | `gtmpl` 严格渲染封装、40 个基础设施辅助函数与实例级 `TemplateRegistry`（Step 01） |
| `src/cert.rs` | 已交付（Phase 0） | ECDSA P-256 证书签发、99 年 `GeneralizedTime`、叶证书截齐与链核验 |
| `src/registry/` | 已交付（Phase 0） | 纯 Rust OCI Distribution v2 拉取/推送、SHA256 全链路校验、Bearer 交换与 Harbor v2 自动建项 |
| `src/prep.rs` | 已交付（Phase 0） | `rtkk prep hosts` 远端 OS 主机名探测与 `/etc/hosts` 标记块同步 |
| `src/logging/` | 已交付（Phase 0） | 文件日志、TTY 进度条及作用域化 `ExecutionEvent` / `EventSink`（Step 01） |
| `src/utils.rs` | 已交付（Phase 0） | 八进制权限解析、Base64 编解码与通用路径工具 |
| `src/workspace.rs` | 规划中（Step 01） | `Workspace` 标准目录树解析、权限收敛与编译期内嵌默认 `configs/` 自举释放 |
| `src/daemon/` | 规划中（Step 02～06） | Axum 常驻 HTTP 服务（`:19912`）、令牌会话鉴权、脱敏器、`JobManager`、SSE 流与内嵌静态资产 |

---

## 2. CLI 顶层子命令全集

| 命令签名 | 状态 | 用途说明 |
| :--- | :--- | :--- |
| `rtkk run <PLAYBOOK> -i <INVENTORY> [-s k=v] [-f N] [--tags t1,t2] [--task-timeout S]` | 已交付 | 并发执行指定 YAML Playbook |
| `rtkk prep hosts -i <INVENTORY> [--best-effort]` | 已交付 | 采集各节点真实主机名并同步集群 `/etc/hosts` 映射块 |
| `rtkk cert gen --cn <CN> --out-key <KEY> --out-cert <CERT> [--ca] [--ca-key K --ca-cert C] [--san S] [--validity V] [--policy P]` | 已交付 | 签发 ECDSA P-256 根 CA 或叶证书（默认 `867240h` = 99 年，私钥 `0600`） |
| `rtkk cert verify <CERT> [--key <KEY>] [--ca <CA_CERT>]` | 已交付 | 校验 X.509 证书有效期、公私钥匹配与上级 CA 签名链 |
| `rtkk pull-images -i <INVENTORY> --images-file <IMAGES> [--output-dir D] [--platform P] [--force] [--insecure]` | 已交付 | 纯 Rust 拉取 OCI 镜像至本地 `docker-legacy` `.tar`（含 `rtkk-media.json`） |
| `rtkk push-images [<TAR>] -i <INVENTORY> [--images-file <IMAGES>] [--output-dir D] [--retag]` | 已交付 | 单包或批量推送本地镜像 `.tar` 至私有镜像仓库（互斥 XOR 模式，支持跨私库自动重标推送） |
| `rtkk sync-images -i <INVENTORY> --images-file <IMAGES> [--output-dir D] [--platform P] [--force] [--insecure]` | 已交付 | 一体化拉取上游镜像并推送至目标私库 |
| `rtkk serve [--listen 127.0.0.1:19912] [--work-dir .] [--token-file .rtkk-daemon/admin.token]` | Step 02 | 启动常驻 HTTP/SSE 管理服务与内嵌 Park UI 控制台 |
| `rtkk healthcheck [--url http://127.0.0.1:19912/api/v1/health]` | Step 02 | 容器与自动化脚本探活子命令（健康返回 `0`，异常返回 `1`） |

---

## 3. Park UI 前端页面路由注册表（`web/src/modules/registry.ts`）

所有导航标签严格遵守 2–4 字简练领域名词规范：

| 路由 Key | URL 路径 | 侧栏导航名 | 鉴权要求 | 交付步骤 |
| :--- | :--- | :--- | :--- | :--- |
| `overview` | `/` | `概览` | 已登录 | Step 03 |
| `inventory` | `/inventory` | `集群配置` | 已登录 | Step 04 |
| `templates` | `/templates` | `模板预览` | 已登录 | Step 04 |
| `artifacts` | `/artifacts` | `离线资源` | 已登录 | Step 05 |
| `operations` | `/operations` | `编排执行` | 已登录 | Step 06 |
| `pki` | `/pki` | `证书备份` | 已登录 | Step 06 |
| `login` | `/login` | —（独立登录页） | 公开 | Step 03 |

---

## 4. 前后端共享枚举字面量契约

Rust `serde` 与前端 `web/src/lib/api/enums.ts` 统一使用小写 `snake_case` 或配置约定的精确字符串字面量：

| 枚举类型 | 允许字面量集合 | 说明 |
| :--- | :--- | :--- |
| `ConnectorType` | `"local"` \| `"ssh"` | 节点执行连接器类型 |
| `BecomeMethod` | `"sudo"` \| `"su"` | 非 `root` 账号提权协议 |
| `HostKeyPolicy` | `"off"` \| `"accept-new"` \| `"strict"` | SSH `known_hosts` 主机公钥校验策略 |
| `CertPolicy` | `"always"` \| `"ifnotpresent"` \| `"none"` | 证书重签与复用策略 |
| `RegistryScheme` | `"auto"` \| `"https"` \| `"http"` | 镜像仓库传输协议模式 |
| `CniPlugin` | `"cilium"` \| `"custom"` \| `"none"` | 集群可插拔 CNI 网络插件选项 |
| `JobStatus` | `"queued"` \| `"running"` \| `"succeeded"` \| `"failed"` \| `"cancelled"` | Daemon 异步任务生命周期状态 |
| `JobConcurrencyClass` | `"mutating"` \| `"readonly"` | 单飞互斥写锁分类 |
| `ExecutionEvent.kind` | `"play_started"` \| `"task_started"` \| `"task_progress"` \| `"task_finished"` \| `"artifact_progress"` \| `"log_line"` | 结构化实时执行事件判别标签 |
| `ApiErrorCode` | `"INVALID_ARGUMENT"` \| `"UNAUTHORIZED"` \| `"NOT_FOUND"` \| `"CONFLICT_ACTIVE_JOB"` \| `"TEMPLATE_RENDER_ERROR"` \| `"AUTH_RATE_LIMITED"` \| `"INTERNAL_ERROR"` \| `"UPSTREAM_UNREACHABLE"` | `/api/v1/*` 统一错误码集合 |

---

## 5. `RTKK_*` 与门禁环境变量全集

| 变量名 | 默认值 | 作用范围与语义 |
| :--- | :--- | :--- |
| `RTKK_MAX_CMD_OUTPUT_BYTES` | `67108864`（64 MiB） | 单次远端/本地命令 `stdout`/`stderr` 最大内存缓冲上限 |
| `RTKK_HTTP_TIMEOUT_SECS` | `600` | `http_get_file` 任务单文件下载超时时间（秒） |
| `RTKK_EMPTY_HOSTS` | `fail` | Play 匹配目标主机为空时的策略（`fail` 或 `skip`/`warn`/`ok`） |
| `RTKK_BIND_ADDR` | `127.0.0.1:19912` | `rtkk serve` 监听地址（容器内 Compose 注入 `0.0.0.0:19912`） |
| `RTKK_WORK_DIR` | `.`（当前目录） | `rtkk serve` 工作区根目录（容器内固定挂载 `/data`） |
| `RTKK_AUTH_TOKEN` | 未设置（自动生成落盘） | 显式注入 Daemon 管理令牌（至少 16 字符） |
| `RTKK_TOKEN_FILE` | `<work_dir>/.rtkk-daemon/admin.token` | 管理令牌持久化文件路径（强制 `0600` 权限） |
| `RTKK_PORT` | `19912` | `deploy/compose/config/.env` 宿主机映射端口 |
| `RTKK_VERSION` | 当前发版号（如 `0.1.0`） | `deploy/compose/config/.env` 引用的 `dex2all/rtkk` 镜像标签 |
| `WT_SLOT` | `0` | 多 `git worktree` 并发门禁槽位号（`0..4` → 端口 `19912` / `29912`～`59912`） |
| `RTKK_ACTIVE_GATES` | `step00` | `scripts/gate/all.sh` 串行执行的激活门禁列表（随 Stage B 演进扩展） |
| `RTKK_VPS_INVENTORY` | 未设置 | `scripts/gate/vps-cluster-e2e.sh` 指向真实 Debian 13 虚拟 VPS 集群的 Inventory 路径 |
