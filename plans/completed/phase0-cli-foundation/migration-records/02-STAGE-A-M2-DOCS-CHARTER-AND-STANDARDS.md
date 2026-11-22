# 02 · Stage A · Step M2：常驻宪章（`docs/`）、活态标准表与实施方案仓（`plans/`）

本步骤建立严格动静分离的文档治理结构：**`docs/`** 存放跨阶段恒定生效的门禁总则、多代理协同宪章、接口规范、5 张活态基准表（`A1`、`A4`、`A5`、`A6`、`A7`）及现有 3 篇运维专题手册；**`plans/`** 存放按阶段演进的分步实施方案与历史完成归档。

---

## M2.1 目标

1. 新建 `docs/QUALITY_GATES_AND_VERIFICATION.md`，定义 G1（静态守卫）、G2（编译与单测）、G3（端到端门禁）三级验证标准。
2. 新建 `docs/MULTI_AGENT_TEAM_WORKFLOW.md`，定义五大角色写入边界白名单、Git Worktree 物理隔离规范与并发测试端口槽（`19912` 主槽 + `29912`～`59912` 并发槽）。
3. 新建 `docs/API_CONTRACT.md` 与 `docs/standards/{A1,A4,A5,A6,A7}.md`，将 `v0.0.5` 已交付的 CLI 能力、公共枚举、依赖基线与可追溯项完整建档，并为 Stage B Daemon API 建立统一协议头与错误体标准。
4. 新建 `plans/README.md` 与 `plans/completed/phase0-cli-foundation/00-PHASE0-BASELINE-ARCHIVE.md`，完成 `v0.0.0`～`v0.0.5` 基线归档。
5. 在 `README.md` 目录与章节中补齐新工程治理文档导航链接，完整保留现有 `OPERATION_GUIDE.md` 与 `docs/*.md` 运维专题。

---

## M2.2 前置条件与负责角色

- **前置条件**：Step M1 已完成。
- **负责角色**：`orchestrator`

---

## M2.3 文件变更清单

| 动作 | 路径 | 说明 |
| :--- | :--- | :--- |
| 新建 | `docs/QUALITY_GATES_AND_VERIFICATION.md` | 三级门禁矩阵、静态守卫清单、Rust/Web 构建测试标准与 `stepNN.sh` 规范 |
| 新建 | `docs/MULTI_AGENT_TEAM_WORKFLOW.md` | 角色独占写入表、L3 子代理职责分离（Research / Author / Verifier / RedTeam）、Worktree 隔离与端口槽分配 |
| 新建 | `docs/API_CONTRACT.md` | Daemon HTTP REST（`/api/v1/*`）与 SSE 流通用契约、会话令牌鉴权、脱敏规则与三字段错误体 `{code, message, request_id}` |
| 新建 | `docs/standards/A1-CONVENTIONS-ENUMS-ROUTES.md` | 模块词表、CLI 子命令表、Daemon 路由前缀规范（`:19912`）、页面注册表预留、公共枚举与 `RTKK_*` 环境变量全集 |
| 新建 | `docs/standards/A4-API-CONTRACT-INDEX.md` | Daemon 路由契约索引表（Stage A 基线为 0 条已挂载路由，随 Stage B Step 02～06 契约 PR 同步登记并由 `check_routes.py` 核验） |
| 新建 | `docs/standards/A5-TEST-FIXTURES-AND-GATES.md` | 确定性测试夹具生成规则（临时证书、离线 OCI tar 包、沙盒 Inventory）与 `scripts/gate/lib.sh` 公共函数规格 |
| 新建 | `docs/standards/A6-DEPENDENCY-BASELINE.md` | 当前 `Cargo.toml` 22 个 Rust 依赖锁定表及 Stage B 准入依赖基线（`axum`、`tower-http`、React 19、`@ark-ui/react`、Tailwind v4） |
| 新建 | `docs/standards/A7-REQUIREMENTS-TRACEABILITY.md` | 需求追踪矩阵：登记 `C01`～`C14`（Phase 0 已完成 CLI 与 Debian 13 编排能力），映射至源码、`test/*.rs` 与 `step00` 断言 |
| 新建 | `plans/README.md` | 实施方案仓总览与动静分离守则 |
| 新建 | `plans/completed/phase0-cli-foundation/00-PHASE0-BASELINE-ARCHIVE.md` | `v0.0.0`～`v0.0.5` 已交付能力、模块边界与 53 项测试基线封存记录 |
| 修改 | `README.md` | 在仓库目录结构与开发门禁章节补充 `IRON_RULES.md`、`docs/QUALITY_GATES_AND_VERIFICATION.md` 与 `plans/` 链接 |

---

## M2.4 核心标准表规格

### M2.4.1 分队角色与独占写入白名单（`docs/MULTI_AGENT_TEAM_WORKFLOW.md`）

| 角色标识（`--role`） | 角色名称 | 独占写入路径白名单 |
| :--- | :--- | :--- |
| `orchestrator` | **L1 架构总控与契约官** | `IRON_RULES.md`、`AGENTS.md`、`GEMINI.md`、`CLAUDE.md`、`README.md`、`OPERATION_GUIDE.md`、`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`、`build.rs`、`src/{lib.rs,main.rs,cli.rs,error.rs}`、`src/daemon/{mod.rs,state.rs,routes/mod.rs}`、`web/{package.json,package-lock.json,tsconfig.json,vite.config.ts}`、`web/src/{App.tsx,lib/api/enums.ts,modules/registry.ts}`、`.github/`、`docs/`、`plans/`、`scripts/*.py`、`scripts/check-write-scope.sh`、`scripts/gate/{lib.sh,all.sh}` |
| `squad-engine` | **L2 核心编排引擎分队** | `src/{cert.rs,prep.rs,utils.rs}`、`src/{connector,inventory,logging,playbook,registry,tasks,template,workspace}/` |
| `squad-daemon` | **L2 常驻服务与接口分队** | `src/daemon/{auth.rs,jobs.rs,mask.rs,sse.rs,embed.rs}`、`src/daemon/routes/`（不含 `routes/mod.rs`） |
| `squad-web` | **L2 Park UI 前端分队** | `web/AGENTS.md`、`web/index.html`、`web/src/{components/,pages/,styles/,hooks/,lib/}`（不含 `App.tsx`、`lib/api/enums.ts`、`modules/registry.ts`） |
| `squad-ops-qa` | **L2 部署配置与门禁验收分队** | `configs/`、`deploy/`、`test/`、`web/e2e/`、`web/playwright.config.ts`、`scripts/gate/step*.sh` |

### M2.4.2 同机多 Worktree 并发端口与目录隔离表

| 槽位标识 | 环境变量 | Daemon 监听地址（`RTKK_BIND_ADDR`） | 隔离工作目录（`RTKK_WORK_DIR`） |
| :--- | :--- | :--- | :--- |
| 主门禁槽（串行 `all.sh`） | `WT_SLOT=0`（默认） | `127.0.0.1:19912` | `.gate/work-slot0` |
| 并发槽 #1 | `WT_SLOT=1` | `127.0.0.1:29912` | `.gate/work-slot1` |
| 并发槽 #2 | `WT_SLOT=2` | `127.0.0.1:39912` | `.gate/work-slot2` |
| 并发槽 #3 | `WT_SLOT=3` | `127.0.0.1:49912` | `.gate/work-slot3` |
| 并发槽 #4 | `WT_SLOT=4` | `127.0.0.1:59912` | `.gate/work-slot4` |

### M2.4.3 Phase 0 基线能力追踪项（`A7` 初始 `C01`～`C14`）

| 编号 | 领域 | 能力描述 | 负责阶段 | 门禁与单测断言 |
| :--- | :--- | :--- | :--- | :--- |
| `C01` | A 连接层 | 本机 `LocalConnector` 与远端 `SshConnector` 连接池、瞬断重连及 `HostKeyPolicy` 校验 | Phase 0 | `step00` `"cli-local-playbook"`；`src/connector/` 单测 |
| `C02` | A 连接层 | 三种节点权限模型：Root SSH 直连、`sudo` 提权、普通用户 `su` PTY 帧隔离密码提权 | Phase 0 | `connector::tests::su_frame_parser_*` |
| `C03` | A 拓扑层 | 扁平与层级双模确定性 Inventory 解析、循环依赖检测与 `localhost` 隔离 | Phase 0 | `test/inventory_tests.rs` |
| `C04` | B 编排层 | 并发 Playbook 引擎（`serial`、`forks`、`when`/`failed_when`/`changed_when`、`loop`、`block/rescue/always`、`roles`） | Phase 0 | `test/playbook_engine_tests.rs` |
| `C05` | B 编排层 | 9 大内置原子任务模块（`gen_cert`、`template`、`copy`、`fetch`、`http_get_file`、`command`/`shell`、`systemd`、`sysctl`、`modprobe`） | Phase 0 | `test/tasks_tests.rs` |
| `C06` | B 模板层 | 严格 `gtmpl` 渲染引擎与 40 个内置基础设施辅助函数 | Phase 0 | `test/template_tests.rs` |
| `C07` | C 安全与 PKI | ECDSA P-256 根 CA 与叶证书签发/链校验、默认 99 年（`867240h`）RFC 5280 `GeneralizedTime` 及 `0600` 权限保护 | Phase 0 | `step00` `"cert-99y-and-verify"`；`test/cert_tests.rs` |
| `C08` | D 镜像同步 | 纯 Rust OCI Distribution v2 拉取、Manifest/Blob 全链路 SHA256 强校验与 `rtkk-media.json` 归档 | Phase 0 | `test/registry_tests.rs` |
| `C09` | D 镜像同步 | OCI V2 Basic/Bearer 挑战应答推送、`scheme` 自动探测与 Harbor v2 项目自动创建 | Phase 0 | `test/registry_tests.rs` |
| `C10` | E 集群交付 | `rtkk prep hosts` 集群 `/etc/hosts` 采集同步及 `deploy.yaml` 幂等保留 OS 主机名 | Phase 0 | `test/prep_and_cli_tests.rs`；`test/configs_render_tests.rs` |
| `C11` | E 集群交付 | Debian 13 离线高可用 K8s `v1.37.2` 全栈编排（外部 etcd `v3.7.4`、containerd `2.4.2` v2/v3 自适应、kube-vip `v1.3.2`） | Phase 0 | `test/configs_render_tests.rs` |
| `C12` | E 集群交付 | 可插拔 CNI 与扩展组件 Role（Cilium `v1.20.5` 离线 Chart、OpenEBS LocalPV、NodeLocalDNS、自定义模板） | Phase 0 | `test/configs_render_tests.rs` |
| `C13` | E 集群交付 | Debian 13 节点准入预检（`--tags check`）、离线 `.deb` + `apt-get` 分级依赖补齐与 `iptables-nft` 规范对齐 | Phase 0 | `test/configs_render_tests.rs` |
| `C14` | F 工程门禁 | G1 静态守卫 + G2 Rust 严格编译文档单测 + G3 `step00` CLI 端到端闭环门禁 | Stage A | `step00` `"cli-version"`、`"fail-fast-bad-inv"` |

---

## M2.5 验收标准

1. `docs/` 下全部新建宪章与 `standards/A1,A4,A5,A6,A7` 完整就位，现有 3 篇运维专题文档零损坏。
2. `plans/README.md` 与 `plans/completed/phase0-cli-foundation/00-PHASE0-BASELINE-ARCHIVE.md` 就位，无任何死链。
