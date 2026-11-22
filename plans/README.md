# RTKK 分阶段演进方案仓（`plans/`）

本目录管理 `rtkk` 按阶段推进的工程实施蓝图与已封板归档记录，与存放跨阶段恒定生效宪章的 [docs/](../docs/QUALITY_GATES_AND_VERIFICATION.md) 保持严格动静分离。

---

## 1. 动静分离治理守则

1. **常驻宪章归入 `docs/`**：全局铁律、三级门禁规则、多代理边界、HTTP/SSE 契约与 `A1`～`A7` 标准表属于长期生效事实源，统一存放于根目录与 `docs/` 下，不在 `plans/` 重复维护副本。
2. **活跃阶段归入 `plans/<phase-name>/`**：当前正在实施或预置门禁的阶段方案存放于 `plans/` 顶层对应阶段子目录，按 `00` 总纲 + `01..NN` 分步契约拆解。
3. **封板归档归入 `plans/completed/`**：当某一阶段全部验收门禁（`scripts/gate/all.sh` 与 Debian 13 CI）全绿合入 `main` 后，将该阶段目录整体迁入 `plans/completed/` 只读封存。

---

## 2. 活跃演进阶段索引

### Phase 1（Stage B）· Daemon（`:19912`）+ 单二进制 Park UI + Compose + Docker Hub 演进

目录：[phase1-daemon-webui-and-compose/](./phase1-daemon-webui-and-compose/00-PHASE1-MASTER-INDEX.md)

| 步骤编号 | 依赖层级 | 实施方案文档 | 绑定门禁脚本 |
| :--- | :--- | :--- | :--- |
| **总纲** | — | [00-PHASE1-MASTER-INDEX.md](./phase1-daemon-webui-and-compose/00-PHASE1-MASTER-INDEX.md) | [scripts/gate/all.sh](../scripts/gate/all.sh) |
| **Step 01** | Layer 1 | [01-STEP01-ENGINE-DECOUPLING-AND-WORKSPACE.md](./phase1-daemon-webui-and-compose/01-STEP01-ENGINE-DECOUPLING-AND-WORKSPACE.md) | [scripts/gate/step01.sh](../scripts/gate/step01.sh) |
| **Step 02** | Layer 2 | [02-STEP02-DAEMON-CORE-AUTH-AND-EMBED.md](./phase1-daemon-webui-and-compose/02-STEP02-DAEMON-CORE-AUTH-AND-EMBED.md) | [scripts/gate/step02.sh](../scripts/gate/step02.sh) |
| **Step 03** | Layer 1 | [03-STEP03-PARKUI-DESIGN-SYSTEM-AND-SHELL.md](./phase1-daemon-webui-and-compose/03-STEP03-PARKUI-DESIGN-SYSTEM-AND-SHELL.md) | [scripts/gate/step03.sh](../scripts/gate/step03.sh) |
| **Step 04** | Layer 3 | [04-STEP04-INVENTORY-STUDIO-AND-PREVIEW.md](./phase1-daemon-webui-and-compose/04-STEP04-INVENTORY-STUDIO-AND-PREVIEW.md) | [scripts/gate/step04.sh](../scripts/gate/step04.sh) |
| **Step 05** | Layer 3 | [05-STEP05-ARTIFACT-PREFETCH-AND-REGISTRY.md](./phase1-daemon-webui-and-compose/05-STEP05-ARTIFACT-PREFETCH-AND-REGISTRY.md) | [scripts/gate/step05.sh](../scripts/gate/step05.sh) |
| **Step 06** | Layer 3 | [06-STEP06-EXECUTION-CONSOLE-PKI-AND-BACKUPS.md](./phase1-daemon-webui-and-compose/06-STEP06-EXECUTION-CONSOLE-PKI-AND-BACKUPS.md) | [scripts/gate/step06.sh](../scripts/gate/step06.sh) |
| **Step 07** | Layer 4 | [07-STEP07-COMPOSE-DOCKERHUB-AND-E2E.md](./phase1-daemon-webui-and-compose/07-STEP07-COMPOSE-DOCKERHUB-AND-E2E.md) | [scripts/gate/step07.sh](../scripts/gate/step07.sh) |
| **VPS 联调** | 实机终验 | [08-VPS-CLUSTER-ACCEPTANCE-MATRIX.md](./phase1-daemon-webui-and-compose/08-VPS-CLUSTER-ACCEPTANCE-MATRIX.md) | [scripts/gate/vps-cluster-e2e.sh](../scripts/gate/vps-cluster-e2e.sh) |

---

## 3. 已完成历史归档索引（`plans/completed/`）

- **[Phase 0 · 纯 Rust CLI 编排底座与 Stage A 门禁治理归档](./completed/phase0-cli-foundation/00-PHASE0-BASELINE-ARCHIVE.md)**：
  - 封存 `v0.0.0` ～ `v0.0.5` 核心连接器、Playbook 引擎、PKI、OCI Registry 同步、Debian 13 高可用 K8s 离线部署配置及 53 项测试基线；
  - 附 Stage A（Step M1 ～ M4）治理迁移归档记录（`migration-records/01..04*.md`）。
