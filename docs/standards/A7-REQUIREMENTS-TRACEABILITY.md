# A7 · 需求与能力追踪矩阵（`C01` ～ `C29`）

本文档建立 `rtkk` 从核心能力编号到源码模块、单元/集成测试及 `scripts/gate/stepNN.sh` 端到端断言的精确映射。[scripts/check_traceability.py](../../scripts/check_traceability.py) 在 G1 门禁阶段自动解析本表所有形如 ``stepNN `"assertion-name"` `` 的标记，并校验对应 `scripts/gate/stepNN.sh` 中存在同名 `run_assertion "assertion-name"` 调用。

---

## 1. Phase 0 与 Stage A 已交付基线能力（`C01` ～ `C14`）

| 编号 | 领域 | 能力规格描述 | 状态 | 核心实现模块 | 单测与端到端门禁断言 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `C01` | A 连接层 | 本机 `LocalConnector` 与远端 `SshConnector` 连接池、瞬断自动重连及 `HostKeyPolicy` 三档指纹校验 | 已交付 | `src/connector/` | step00 `"cli-local-playbook"`；`src/connector/` 单测 |
| `C02` | A 连接层 | 三种节点权限模型：Root SSH 直连、`sudo` 提权、普通用户 `su` PTY 帧隔离密码提权 | 已交付 | `src/connector/ssh.rs` | `connector::tests::su_frame_parser_*`；`connector::tests::become_method_*` |
| `C03` | A 拓扑层 | 扁平与层级双模确定性 Inventory 解析、有向图循环依赖拦截与 `localhost` 组隔离 | 已交付 | `src/inventory/` | `test/inventory_tests.rs` |
| `C04` | B 编排层 | 并发 Playbook 引擎（`serial` 分批、`forks` 限流、`when`/`failed_when`/`changed_when`、`loop`、`block/rescue/always`、`roles`） | 已交付 | `src/playbook/` | `test/playbook_engine_tests.rs` |
| `C05` | B 编排层 | 9 大内置原子任务模块（`gen_cert`、`template`、`copy`、`fetch`、`http_get_file`、`command`/`shell`、`systemd`、`sysctl`、`modprobe`） | 已交付 | `src/tasks/` | `test/tasks_tests.rs` |
| `C06` | B 模板层 | 严格 `gtmpl` 渲染引擎与 40 个内置基础设施辅助函数（缺失键安全求值与语法错误快速失败） | 已交付 | `src/template/` | `test/template_tests.rs` |
| `C07` | C 安全与 PKI | ECDSA P-256 根 CA 与叶证书签发/链校验、默认 99 年（`867240h`）RFC 5280 `GeneralizedTime` 及 `0600` 私钥权限保护 | 已交付 | `src/cert.rs` | step00 `"cert-99y-and-verify"`；`test/cert_tests.rs` |
| `C08` | D 镜像同步 | 纯 Rust OCI Distribution v2 拉取、Manifest/Blob 全链路 SHA256 强校验与 `rtkk-media.json` 媒体类型保真 | 已交付 | `src/registry/pull.rs` | `test/registry_tests.rs` |
| `C09` | D 镜像同步 | OCI V2 Basic/Bearer 挑战应答推送、`scheme` 自动探测与 Harbor v2 项目自动创建 | 已交付 | `src/registry/client.rs` | `test/registry_tests.rs` |
| `C10` | E 集群交付 | `rtkk prep hosts` 集群 `/etc/hosts` 采集同步及 `deploy.yaml` 幂等保留 OS 主机名 | 已交付 | `src/prep.rs`、`configs/deploy.yaml` | `test/prep_and_cli_tests.rs`；`test/configs_render_tests.rs` |
| `C11` | E 集群交付 | Debian 13 离线高可用 K8s `v1.37.2` 全栈编排（外部 etcd `v3.7.4`、containerd `2.4.2` v2/v3 自适应、kube-vip `v1.3.2`） | 已交付 | `configs/deploy.yaml`、`configs/templates/` | `test/configs_render_tests.rs` |
| `C12` | E 集群交付 | 可插拔 CNI 与扩展组件 Role（Cilium `v1.20.5` 离线 Chart、OpenEBS LocalPV、NodeLocalDNS、自定义模板） | 已交付 | `configs/roles/` | `test/configs_render_tests.rs` |
| `C13` | E 集群交付 | Debian 13 节点准入预检（`--tags check`）、离线 `.deb` + `apt-get` 分级依赖补齐与 `iptables-nft` 规范对齐 | 已交付 | `configs/roles/debian13-preflight/` | `test/configs_render_tests.rs` |
| `C14` | F 工程门禁 | G1 七项静态守卫 + G2 Rust 严格编译文档单测 + G3 `step00` CLI 端到端闭环门禁 | 已交付（Stage A） | `scripts/*.py`、`scripts/gate/` | step00 `"cli-version"`、step00 `"fail-fast-bad-inv"` |

---

## 2. Stage B 能力追踪矩阵（`C15` ～ `C29`）

`C15` ～ `C29`（Step 01 ～ Step 07）已完成代码交付并通过 `step01.sh` ～ `step07.sh` 端到端验收：

| 编号 | 领域 | 能力规格描述 | 状态 / 步骤 | 核心实现模块 | 绑定门禁脚本断言 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `C15` | B 编排层 | 多任务并发作用域 `EventSink` 与实例级 `TemplateRegistry` 零串扰隔离 | 已交付（Step 01） | `src/logging/events.rs`、`src/template/mod.rs` | step01 `"concurrent-sink-isolated"` |
| `C16` | G 服务底座 | `Workspace` 标准目录解析、空工作区自动释放内嵌默认 `configs/` 且幂等保留用户改动 | 已交付（Step 01） | `src/workspace.rs` | step01 `"workspace-bootstrap-idempotent"` |
| `C17` | G 服务底座 | `rtkk serve`（`:19912`）与 `rtkk healthcheck` 探活生命周期、令牌 `0600` 自举及优雅停机 | 已交付（Step 02） | `src/cli.rs`、`src/daemon/mod.rs` | step02 `"daemon-healthcheck-ok"` |
| `C18` | C 安全与鉴权 | 未授权请求 `/api/v1/*` 强制 `401` JSON 阻断、常量时间令牌登录与 `HttpOnly` 会话闭环 | 已交付（Step 02） | `src/daemon/auth.rs`、`src/daemon/routes/auth.rs` | step02 `"unauth-401-blocked"`、step02 `"login-session-ok"` |
| `C19` | G 服务底座 | 集群写任务全局单飞互斥锁（`409 CONFLICT_ACTIVE_JOB`）与 SSE 历史 JSONL 回放+实时尾随 | 已交付（Step 02） | `src/daemon/jobs.rs`、`src/daemon/sse.rs` | step02 `"mutating-job-mutex"`、step02 `"sse-replay-complete"` |
| `C20` | H Park UI | Park UI Light/Dark 双色对比度、在线 Theme Editor 与纯离线单二进制内嵌 SPA（Gzip + ETag `304`） | 已交付（Step 03） | `build.rs`、`src/daemon/embed.rs`、`web/` | step03 `"spa-embedded-index"`、step03 `"spa-gzip-etag"` |
| `C21` | C 安全与配置 | Inventory API 敏感字段 `********` 脱敏读取、未改密文无损保留回写与 `auth_basic` 自动重算 | 已交付（Step 04） | `src/daemon/mask.rs`、`src/daemon/routes/inventory.rs` | step04 `"inventory-secret-masked"`、step04 `"inventory-preserve-unmodified-secret"` |
| `C22` | E 集群交付 | 公网域名 DNS 劫持免证书开关（`public_registries_insecure`）双栈 `hosts.toml` 渲染 | 已交付（Step 04） | `configs/deploy.yaml`、`configs/inventory.yaml` | step04 `"public-registries-hijack-hosts-toml"` |
| `C23` | H 工作台 | 指定目标节点 6 大 `.gtmpl` 配置文件及 `images.yaml` 在线实时渲染预览与语法核验 | 已交付（Step 04） | `src/daemon/routes/templates.rs`、`web/src/pages/TemplatesPage.tsx` | step04 `"template-preview-render-ok"` |
| `C24` | D 离线与私库 | 离线产物目录落盘校验态盘点并与 `inventory.yaml` / `images.yaml` 动态清单精确对齐 | 已交付（Step 05） | `src/daemon/routes/artifacts.rs`、`web/src/pages/ArtifactsPage.tsx` | step05 `"artifacts-status-inventory-aligned"` |
| `C25` | D 离线与私库 | Daemon API 在线探测 Harbor/OCI 连通性并异步驱动镜像拉取、建项与批量推送闭环 | 已交付（Step 05） | `src/daemon/routes/artifacts.rs`、`src/registry/` | step05 `"daemon-registry-ping-and-sync"` |
| `C26` | B 编排控制台 | Playbook 大纲静态解析与通过 API 触发带 `--tags` 过滤的异步编排执行闭环 | 已交付（Step 06） | `src/daemon/routes/operations.rs`、`web/src/pages/OperationsPage.tsx` | step06 `"playbook-outline-and-tagged-run"` |
| `C27` | C PKI 与备份 | Web API 在线签发 99y/1y X.509 证书、公开元数据读取（零私钥泄露）、链校验与 `rtkk_bak/` 流式导出 | 已交付（Step 06） | `src/daemon/routes/pki.rs`、`web/src/pages/PkiPage.tsx` | step06 `"pki-api-gen-inspect-verify"`、step06 `"backup-archive-stream"` |
| `C28` | I 容器与发布 | `debian:13-slim` 非特权容器镜像（`uid=10001`）、独立 `deploy/compose/`（`:19912`）一键启停及 `dex2all/rtkk` 发布 | 已交付（Step 07） | `deploy/`、`.github/workflows/release.yml` | step07 `"compose-lifecycle-19912"` |
| `C29` | H Park UI 终验 | 纯断网离线拦截下 Playwright 验证 Light/Dark `>= 4.5:1` 对比度、零 `<select>`、全键盘无障碍、`<768px` 零溢出与全旅程 | 已交付（Step 07） | `web/e2e/parkui-and-journey.spec.ts` | step07 `"playwright-parkui-offline-e2e"` |
