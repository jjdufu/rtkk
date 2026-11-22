# Phase 0 基线封存记录：`rtkk v0.0.0` ～ `v0.0.5` 纯 Rust CLI 编排底座与 Stage A 门禁迁移

本归档固化 `rtkk` 在进入 Phase 1（Daemon + 内嵌 Park UI + Compose）演进前已完成的全部 CLI 核心能力、Debian 13 离线高可用 Kubernetes 部署拓扑、53 项自动化测试套件及 Stage A（Step M1 ～ M4）治理底座迁移记录。

---

## 1. 基线版本与工具链快照

| 维度 | 锁定事实 |
| :--- | :--- |
| **基线版本** | `0.0.5`（Tag `v0.0.5`） |
| **Rust 工具链** | `1.100.0`（Edition `2024`） |
| **测试目标环境** | Debian 13 (`Trixie`) `x86_64` |
| **静态安全契约** | `unsafe_code = "forbid"`，全量 `rustc` / `clippy::pedantic` / `missing_docs` / `unwrap_used` / `expect_used` / `as_conversions` / `indexing_slicing` 设为 `deny` |
| **已交付能力范围** | `C01` ～ `C14`（详见 [A7-REQUIREMENTS-TRACEABILITY.md](../../../docs/standards/A7-REQUIREMENTS-TRACEABILITY.md)） |

---

## 2. Phase 0 已交付核心子系统清单

1. **连接与提权层（`src/connector/`）**：
   - `LocalConnector`（`shell-words` 无 Shell 参数直调 + `/bin/sh -c` 模式）与 `SshConnector`（`OnceCell` 连接池复用 + 瞬断自动重建）；
   - 四级有序身份认证、三档 `HostKeyPolicy`（`off` / `accept-new` / `strict`），以及 `sudo` 与普通用户 `su` PTY 帧隔离提权。
2. **确定性拓扑与编排引擎（`src/inventory/`、`src/playbook/`、`src/tasks/`、`src/template/`）**：
   - 扁平与层级双模 Inventory、DAG 循环检测、`localhost` 隔离及集群动态变量自动注入；
   - 支持 `serial`、`forks`、`when`/`failed_when`/`changed_when`、`loop`/`with_items`、`block`/`rescue`/`always`、`roles/` 展开的并发执行引擎；
   - 9 大内置任务模块与 40 个 `gtmpl` 基础设施辅助函数。
3. **内置 PKI 与纯 Rust OCI 镜像同步（`src/cert.rs`、`src/registry/`）**：
   - ECDSA P-256 根 CA 与叶证书签发/校验，默认 99 年（`867240h`）RFC 5280 `GeneralizedTime`、`0600` 私钥落盘与上级 CA 有效期截齐；
   - 纯 Rust OCI Distribution v2 拉取（`docker-legacy` `.tar` + `rtkk-media.json` + 全链路 SHA256 强校验）与推送（`Basic` / `401 Bearer` 自动换票 + Harbor v2 项目自动创建）。
4. **Debian 13 离线高可用 K8s 交付套件（`configs/`）**：
   - 覆盖 K8s `v1.37.2`、外部高可用 etcd `v3.7.4`、containerd `2.4.2`（v2/v3 配置模板自适应）、kube-vip `v1.3.2`、Cilium `v1.20.5` 离线 Chart、OpenEBS LocalPV `4.6.0`、NodeLocalDNS `1.27.1` 及 Debian 13 节点预检（`--tags check` / `preflight`）。

---

## 3. 存量 53 项自动化测试基线分布

| 测试载体 | 用例数量 | 覆盖重点 |
| :--- | :---: | :--- |
| 库内单元测试（`src/lib.rs` 各子模块 `mod tests`） | 11 | `su` PTY 帧二进制无损解析、`BecomeMethod` 解析、`WWW-Authenticate` 引号逗号解析、`auth_basic` 派生、模板缺失键与辅助函数 |
| `test/cert_tests.rs` | 3 | 99 年/1 年/多年期时长解析、叶证书截齐 CA 上限、`GeneralizedTime` 编码与 `0600` 权限 |
| `test/configs_render_tests.rs` | 11 | 全量真实 Playbook 编译、3M+3W 高可用/单节点/独立外部 etcd 拓扑渲染、containerd v2/v3 自动切换、Debian 13 预检 Role 渲染 |
| `test/inventory_tests.rs` | 4 | 循环组依赖拦截、层级空子节点兼容、`localhost` 排除出 `all` 组与确定性排序 |
| `test/playbook_engine_tests.rs` | 7 | 动态变量注入、严格条件求值、`loop` 聚合、`block/rescue/always` 恢复、`serial` 下 `run_once` 唯一性 |
| `test/prep_and_cli_tests.rs` | 6 | 全子命令参数解析、`push-images` XOR 校验、`--forks 0` 防死锁、`-s msg=help` 透传、CLI 证书签发核验闭环 |
| `test/registry_tests.rs` | 3 | 篡改 Manifest/Blob SHA256 拒绝、本地 tar 幂等跳过与标签变更重拉、Bearer 逗号 Scope 与 Harbor 自动建项推送 |
| `test/tasks_tests.rs` | 6 | `copy`、`template`、`fetch`、`command`/`shell`、`gen_cert`、`http_get_file` 幂等性与异常清理 |
| `test/template_tests.rs` | 2 | `<no value>` 安全分支处理与 `b64enc`、`semverCompare`、数学/集合辅助函数 |
| **合计** | **53** | **全套 `cargo test --workspace --locked` 100% 通过** |

---

## 4. Stage A 工程治理底座迁移记录索引

Stage A（Step M1 ～ Step M4）分步迁移方案归档于 [migration-records/](./migration-records/01-STAGE-A-M1-MEMORY-AND-IRON-RULES.md)：

1. [01-STAGE-A-M1-MEMORY-AND-IRON-RULES.md](./migration-records/01-STAGE-A-M1-MEMORY-AND-IRON-RULES.md)
2. [02-STAGE-A-M2-DOCS-CHARTER-AND-STANDARDS.md](./migration-records/02-STAGE-A-M2-DOCS-CHARTER-AND-STANDARDS.md)
3. [03-STAGE-A-M3-STATIC-GUARDS-AND-STEP00-GATE.md](./migration-records/03-STAGE-A-M3-STATIC-GUARDS-AND-STEP00-GATE.md)
4. [04-STAGE-A-M4-DEBIAN13-CI-AND-CUTOVER.md](./migration-records/04-STAGE-A-M4-DEBIAN13-CI-AND-CUTOVER.md)
