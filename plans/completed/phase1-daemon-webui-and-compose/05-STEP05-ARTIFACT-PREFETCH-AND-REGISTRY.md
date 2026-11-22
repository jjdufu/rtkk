# 05 · Stage B · Step 05：离线资源预下载与私有镜像仓库同步中心

本步骤将 `configs/download.yaml`、`configs/images.yaml` 与 `src/registry/` 的离线制品管理能力转化为可视化资产中心：支持实时清点二进制、压缩包、Debian 13 `.deb` 离线依赖归档及容器镜像 `.tar` 的落盘状态，支持联网准备阶段按需异步预下载，并在内网阶段一键探测 Harbor 私库连通性、自动创建项目及按子集批量推送镜像。

---

## 1. 目标与边界

1. **动态清单对齐盘点**：扫描 `<work_dir>/downloads/{bin,tar,images}`，结合当前 `configs/inventory.yaml` 组件版本与动态渲染后的 `configs/images.yaml` 镜像列表，输出每项资产的存在性、字节体积、修改时间、归档内 `RepoTags` 与 `rtkk-media.json` 完整性状态。
2. **细粒度异步传输调度**：通过 `JobManager`（`mutating` 排他锁）异步驱动二进制/归档预下载（调用 `download.yaml` 指定 Tags）、上游镜像勾选拉取（`registry::pull`）与内网私库推送（`registry::push`，含 Harbor v2 项目自动创建与 Bearer Token 交换），全程通过 SSE 实时推送 `ArtifactProgress` 字节进度帧。
3. **无副作用私库连通性诊断**：提供 `POST /api/v1/registry/ping` 只读探测接口，验证 `scheme: auto | https | http` 协商、TLS 根证书校验、`401 Basic / Bearer` 令牌换票及 Harbor 项目存在性，不向远端写入任何 Blob。

---

## 2. 分队职责与文件变更矩阵

| 负责角色 | 动作 | 文件路径 | 变更内容说明 |
| :--- | :---: | :--- | :--- |
| `orchestrator` | 修改 | `src/daemon/routes/mod.rs`、[A4](../../../docs/standards/A4-API-CONTRACT-INDEX.md)、`web/src/App.tsx` | 挂载 Step 05 共 5 条路由并迁入 `A4` 已挂载表；绑定 `/artifacts` 页面路由 |
| `squad-engine` | 修改 | `src/registry/{mod.rs,pull.rs,client.rs}` | 暴露支持 `image_names` 子集过滤的库级拉取/推送入口与只读 `ping_registry` 诊断函数 |
| `squad-daemon` | 新建 | `src/daemon/routes/artifacts.rs` | 实现离线产物盘点、异步下载触发、私库连通性诊断及镜像拉取/推送路由 |
| `squad-web` | 新建 | `web/src/lib/api/artifacts.ts` | 离线资产状态与镜像同步任务 API 客户端 |
| `squad-web` | 新建 | `web/src/pages/ArtifactsPage.tsx` | 顶部三卡片统计栏、双标签页资产表格（含可见行多选/半选）与实时传输进度抽屉 |
| `squad-ops-qa` | 已预置 | [scripts/gate/step05.sh](../../../scripts/gate/step05.sh) | G3 门禁脚本（内置本地 Python Mock OCI v2 + Harbor v2 服务验证 `C24`、`C25`） |

---

## 3. HTTP 接口与前端交互契约

### 3.1 本步挂载的 5 条接口规格

| 方法与路径 | 鉴权 / 互斥 | 请求体规格 | 成功响应规格 |
| :--- | :---: | :--- | :--- |
| `GET /api/v1/artifacts/status` | 已登录 / 只读 | 无 | `{"summary":{"binaries_ready":5,"binaries_total":5,"archives_ready":5,"archives_total":5,"debs_bundle_ready":false,"images_ready":12,"images_total":12,"total_bytes":1081081856},"binaries":[ArtifactItem],"archives":[ArtifactItem],"images":[ImageArtifactItem]}` |
| `POST /api/v1/artifacts/download` | 已登录 / 变更 | `{"scopes":["binaries","archives","charts","debs"],"force":false}` | `{"job_id":"job_...","status":"running"}` |
| `POST /api/v1/registry/ping` | 已登录 / 只读 | `{"registry_override":null}`（可选候选私库参数） | `{"reachable":true,"resolved_scheme":"https","auth_mode":"bearer","is_harbor":true,"project":"rtkk","project_exists":true,"latency_ms":24}` |
| `POST /api/v1/registry/pull` | 已登录 / 变更 | `{"image_names":["pause","etcd"],"force":false,"insecure":false}`（空数组表示全量） | `{"job_id":"job_...","status":"running"}` |
| `POST /api/v1/registry/push` | 已登录 / 变更 | `{"image_names":[]}`（空数组表示全量已就绪镜像） | `{"job_id":"job_...","status":"running"}` |

### 3.2 前端视图与微文案契约（`ArtifactsPage.tsx`）

1. **三卡片汇总栏（强制 `.tabular-nums`）**：
   - `二进制与归档`：展示就绪比与累计字节体积（如 `10 / 10 · 248.6 MiB`）；
   - `Debian 13 依赖包`：区分 `离线包就绪`（检测到 `downloads/tar/debian13-deps.tar.gz`）与 `在线 APT 模式`；
   - `容器镜像归档`：展示 `images.yaml` 生效条目就绪比与总体积（如 `12 / 12 · 782.4 MiB`）。
2. **双标签页切换（`@ark-ui/react` `Tabs`）**：`组件与离线包`、`容器镜像清单`（支持搜索过滤与可见行全选/`indeterminate` 半选）。
3. **动作按钮微文案**：严格限定为 `探测私库`、`下载缺失项`、`拉取镜像`、`推送私库`、`强制重拉`、`取消任务`。

---

## 4. 对抗审查要点（Verifier & RedTeam Checklist）

1. **路径穿越拦截**：校验 `image_names` 与归档文件名严禁包含 `..`、`/` 或 `\`，复用 `src/registry/image_list.rs` 安全文件名规则。
2. **凭据零回传**：当调用 `POST /api/v1/registry/ping` 传入 `"password": "********"` 时，服务端自动从磁盘 `configs/inventory.yaml` 读取真实私库口令发起挑战应答握手，绝不在诊断结果或错误消息中回显 `Authorization` 头原文。

---

## 5. 门禁验收标准（[scripts/gate/step05.sh](../../../scripts/gate/step05.sh)）

- `python3 scripts/check_routes.py` 核验累计 19 条已挂载路由双向对齐。
- 追踪编号：`C24`（`"artifacts-status-inventory-aligned"`）、`C25`（`"daemon-registry-ping-and-sync"`）。
- `step05.sh` 末行输出 `GATE step05 PASS`。
