# 06 · Stage B · Step 06：Playbook 实时编排执行控制台与 PKI · 备份中心

本步骤交付集群全生命周期编排操作台（`OperationsPage.tsx`）与证书/备份管理台（`PkiPage.tsx`），将 `prep hosts`、只读体检（`--tags check`）、基线收敛（`--tags preflight`）、全栈部署（`deploy.yaml`）及集群重置（`cleanup.yaml`）呈现为实时 Play × Task × Host 状态矩阵，并提供 X.509 证书在线签发/核验与 `rtkk_bak/` 状态归档流式下载。

---

## 1. 目标与边界

1. **静态大纲解析与受控编排触发**：解析内置 Playbook 的 Play/Task/Tag 拓扑供前端按需勾选执行范围；对 `deploy.yaml --tags check` 免除排他写锁，对 `prep-hosts`、`preflight`、完整 `deploy` 与 `cleanup` 严格施加单飞互斥锁。
2. **智能滚动锁实时日志控制台**：左侧以树状进度展示 Play × Task × Host 成功/变更/跳过/失败计数，右侧按节点过滤流式展示 `stdout` / `stderr` 与毫秒耗时；滚动条距底部 `< 24px` 时自动跟随新日志，用户上翻排查时立即锁定视口并浮现 `回到底部` 轻按钮。
3. **PKI 在线管理与零私钥泄漏**：扫描 `<work_dir>/pki/` 公开 `.crt` 文件输出 Subject CN、Issuer、SAN 列表、起止时间与剩余天数（**严禁**暴露任何 `.key` 内容）；支持在线签发 99 年（`867240h`）或 1 年期 ECDSA P-256 证书落盘至 `pki/custom/`（私钥强制 `0600`）及链校验。
4. **集群状态备份浏览与流式打包**：展示 `<work_dir>/rtkk_bak/` 下部署收尾回收的节点配置与 `admin.conf` 元数据，支持通过 `flate2` + `tar` 动态流式导出 `.tar.gz` 归档包并记录审计事件。

---

## 2. 分队职责与文件变更矩阵

| 负责角色 | 动作 | 文件路径 | 变更内容说明 |
| :--- | :---: | :--- | :--- |
| `orchestrator` | 修改 | `src/daemon/routes/mod.rs`、[A4](../../../docs/standards/A4-API-CONTRACT-INDEX.md)、`web/src/App.tsx` | 挂载 Step 06 共 8 条路由（累计全部 27 条挂载完毕）；绑定 `/operations` 与 `/pki` 页面路由 |
| `squad-engine` | 修改 | `src/playbook/parser.rs` | 暴露只读 `extract_playbook_outline` 函数，返回各 Play 名称、目标选择器、展开后 Task 名与去重 Tags 列表 |
| `squad-daemon` | 新建 | `src/daemon/routes/{operations.rs,pki.rs}` | 实现编排触发、大纲查询、PKI 扫描/签发/链校验及 `rtkk_bak/` 流式归档导出接口 |
| `squad-web` | 新建 | `web/src/lib/api/{operations.ts,pki.ts}` | 编排调度、Playbook 大纲、证书元数据与备份下载 API 客户端 |
| `squad-web` | 新建 | `web/src/pages/{OperationsPage,PkiPage}.tsx` | 快捷动作栏、Play × Task × Host 矩阵、智能滚底日志流、高危二次确认弹窗及证书/备份面板 |
| `squad-ops-qa` | 已预置 | [scripts/gate/step06.sh](../../../scripts/gate/step06.sh) | G3 门禁脚本（覆盖 `C26`、`C27`） |

---

## 3. HTTP 接口与高危防护契约

### 3.1 本步挂载的 8 条接口规格

| 方法与路径 | 鉴权 / 互斥 | 请求体或参数规格 | 成功响应规格 |
| :--- | :---: | :--- | :--- |
| `POST /api/v1/operations/prep-hosts` | 已登录 / 变更 | `{"best_effort":false}` | `{"job_id":"job_...","status":"running"}` |
| `POST /api/v1/operations/run` | 已登录 / 动态 | `{"playbook":"deploy"\|"cleanup"\|"download","tags":["check"],"forks":20,"extra_vars":{}}` | `{"job_id":"job_...","status":"running"}` |
| `GET /api/v1/playbooks/{name}/outline` | 已登录 / 只读 | `name` 限 `"deploy"` \| `"cleanup"` \| `"download"` | `{"name":"deploy","all_tags":["check","preflight","baseline"],"plays":[{"name":"Play 1...","hosts":"all","tasks":[{"name":"...","tags":["preflight"]}]}]}` |
| `GET /api/v1/pki/certificates` | 已登录 / 只读 | 无 | `{"items":[{"rel_path":"ca.crt","cn":"kubernetes","issuer":"kubernetes","is_ca":true,"sans":[],"not_before":"...","not_after":"...","days_remaining":36134,"has_matching_key":true}]}` |
| `POST /api/v1/pki/generate` | 已登录 / 变更 | `{"name":"harbor-tls","cn":"10.0.0.10","is_ca":false,"ca_name":"registry-ca","sans":["10.0.0.10","harbor.internal"],"validity":"867240h"}` | `{"cert_rel_path":"custom/harbor-tls.crt","key_rel_path":"custom/harbor-tls.key","validity_hours":867240}` |
| `POST /api/v1/pki/verify` | 已登录 / 只读 | `{"cert_rel_path":"custom/harbor-tls.crt","key_rel_path":"custom/harbor-tls.key","ca_rel_path":"custom/registry-ca.crt"}` | `{"valid":true,"report":"..."}` |
| `GET /api/v1/backups` | 已登录 / 只读 | 无 | `{"items":[{"rel_path":"c23-p61/etc/kubernetes/admin.conf","size_bytes":5642,"modified_ms":...}]}` |
| `GET /api/v1/backups/archive` | 已登录 / 只读 | 无 | `application/gzip` 流式响应（`Content-Disposition: attachment; filename="rtkk-backups-<ts>.tar.gz"`） |

### 3.2 高危动作确认与按钮微文案契约

- **快捷编排动作按钮**：`同步主机名`、`环境预检`、`收敛基线`、`部署集群`、`清理集群`。
- **破坏性操作门锁**：触发 `cleanup` 或在 `extra_vars` 开启 `force_reinstall: true` 时，前端弹出基于 `@ark-ui/react` `Dialog` 的模态确认框，正文明确列出将执行 `kubeadm reset` 并移除 `/var/lib/etcd` 数据且要求输入确认词 `CLEANUP`，底部按钮严格使用 `取消` / `确认清理`。

---

## 4. 对抗审查要点（Verifier & RedTeam Checklist）

1. **白名单 Playbook 约束**：`POST /api/v1/operations/run` 与 `GET /api/v1/playbooks/{name}/outline` 仅允许枚举字面量 `"deploy"`、`"cleanup"`、`"download"`（开发环境门禁允许沙盒工作区内的 `"sandbox"`），严禁接受任意文件系统路径以防越权执行工作区外 YAML。
2. **零私钥下载面**：`GET /api/v1/pki/certificates` 仅扫描 `.crt` / `.pem` 证书公钥块，绝不读取或拼装 `.key` 文件字节；生成的 `pki/custom/*.key` 落盘权限强制校验为 `0600`。

---

## 5. 门禁验收标准（[scripts/gate/step06.sh](../../../scripts/gate/step06.sh)）

- `python3 scripts/check_routes.py` 双向核验全部 27 条已挂载路由零偏差。
- 追踪编号：`C26`（`"playbook-outline-and-tagged-run"`）、`C27`（`"pki-api-gen-inspect-verify"`、`"backup-archive-stream"`）。
- `step06.sh` 末行输出 `GATE step06 PASS`。
