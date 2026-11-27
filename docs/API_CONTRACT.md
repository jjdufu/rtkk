# Daemon HTTP REST 与 SSE 实时事件流接口总契约

本文档定义 `rtkk serve` 常驻守护进程（默认监听 `127.0.0.1:19912`）的传输约定、高熵令牌会话鉴权、敏感凭据脱敏写回协议、单飞互斥锁规则、统一错误响应体及 SSE 断点回放规范。具体路由清单以 [A4-API-CONTRACT-INDEX.md](./standards/A4-API-CONTRACT-INDEX.md) 为准。

---

## 1. 监听端点、路由命名空间与静态回退边界

| 请求路径前缀 | 响应行为 |
| :--- | :--- |
| `/api/v1/*` | 结构化 JSON API 或 `text/event-stream` SSE 端点；所有响应携带 `X-Request-Id: req_<hex>` 追踪头 |
| `/api/*`（未匹配路由） | 强制返回 HTTP `404 Not Found` 结构化 JSON 错误体，**严禁**回退为 HTML 页面 |
| `/assets/*`、`/favicon.svg` | 内嵌前端编译静态产物，协商 `Accept-Encoding: gzip` 并返回强校验 `ETag: "<sha256>"` 与 `Cache-Control: public, max-age=31536000, immutable`（命中 `If-None-Match` 返回 `304 Not Modified`） |
| 其余非 `/api/*` 的 `GET` / `HEAD` 请求 | 回退返回内嵌 SPA 入口 `index.html`（`Cache-Control: no-cache`），支撑前端路由（`/inventory`、`/templates`、`/artifacts`、`/harbor`、`/operations`、`/pki`、`/login`）直接刷新 |

---

## 2. 访问令牌初始化与双通道会话鉴权

### 2.1 高熵管理令牌引导

`rtkk serve` 启动时按以下优先级加载管理令牌：
1. 环境变量 `RTKK_AUTH_TOKEN`（非空且长度 `>= 16` 字节）；
2. `--token-file` 或 `<work_dir>/.rtkk-daemon/admin.token` 已有文件内容；
3. 若上述均未提供，使用操作系统 CSPRNG 生成 32 字节随机熵（编码为 43 字符无填充 Base64URL），创建 `0700` 目录 `<work_dir>/.rtkk-daemon/` 并以 `0600` 权限原子写入 `admin.token`。

### 2.2 鉴权通道与常量时间校验

除公开探活端点 `GET /api/v1/health` 与登录端点 `POST /api/v1/auth/login` 外，其余全部 `/api/v1/*` 接口（含 SSE 流 `GET /api/v1/jobs/{id}/events`）强制要求已认证身份，支持两种等效凭据：

1. **浏览器会话 Cookie（WebUI 默认）**：
   - 调用 `POST /api/v1/auth/login` 提交 `{"token": "<admin_token>"}`；
   - 服务端使用 `subtle::ConstantTimeEq` 执行常量时间字节比对，校验通过后签发随机 32 字节会话 ID，设置响应头：
     `Set-Cookie: rtkk_session=<sid>; HttpOnly; SameSite=Strict; Path=/; Max-Age=43200`（当检测到 `X-Forwarded-Proto: https` 或 TLS 连接时追加 `Secure`）。
2. **脚本 / CLI Bearer 头**：
   - 请求头携带 `Authorization: Bearer <admin_token>`，服务端同样执行常量时间比对。

未携带凭据、会话过期或令牌不匹配时，统一返回 HTTP `401 Unauthorized` JSON 错误体。

---

## 3. 统一错误响应体规范

所有非 `2xx` 的 `/api/*` 响应固定使用 `Content-Type: application/json; charset=utf-8`，精确包含三个顶层字段：

```json
{
  "code": "CONFLICT_ACTIVE_JOB",
  "message": "Mutating job job_0193a8f2c1 is currently running in this workspace",
  "request_id": "req_9b41e702a3c8"
}
```

### 标准错误码字典

| HTTP 状态码 | `code` 枚举值 | 触发条件 |
| :--- | :--- | :--- |
| `400` | `INVALID_ARGUMENT` | 请求 JSON 语法非法、参数缺失、CIDR/IP/版本号校验失败或 Inventory 存在循环组依赖 |
| `401` | `UNAUTHORIZED` | 未提供有效会话 Cookie / Bearer 令牌，或登录令牌错误 |
| `404` | `NOT_FOUND` | 未知 `/api/*` 路径，或指定的 `job_id` / Playbook / 证书名称不存在 |
| `409` | `CONFLICT_ACTIVE_JOB` | 当前工作区已有运行中的集群变更型任务，拒绝并发写操作 |
| `422` | `TEMPLATE_RENDER_ERROR` | `.gtmpl` 模板语法错误、条件求值失败或渲染产物 TOML/YAML 校验不通过 |
| `429` | `AUTH_RATE_LIMITED` | 连续登录失败触发源地址退避限流保护 |
| `500` | `INTERNAL_ERROR` | 工作区磁盘 I/O 异常或未预期的内部执行错误 |
| `502` | `UPSTREAM_UNREACHABLE` | 在线探测目标 SSH 节点或远端 OCI / Harbor 仓库网络不可达 |

---

## 4. 敏感凭据掩码脱敏与无损回写协议

1. **读取必脱敏**：`GET /api/v1/inventory` 等配置读取接口在序列化响应前，递归将以下敏感路径的值替换为固定掩码常量 `"********"`（原始结构与 YAML 预览文本同步脱敏）：
   - `vars.ansible_ssh_pass`、`vars.ansible_become_pass`、`vars.ansible_become_password`、`vars.become_pass`
   - `vars.registry.password`、`vars.registry.auth_basic`
   - `vars.image_sync.source_auths.*.password`
   - `hosts.*.ansible_ssh_pass`、`hosts.*.ansible_become_pass` 及嵌套组 `vars` 中的同名字段
   - 任何内联私钥文本字段（`private_key_content`）；PKI 接口仅返回公开证书 X.509 元数据，严禁在任何 HTTP 响应中返回 `.key` 私钥内容。
2. **未改密文无损保留**：客户端调用 `PUT /api/v1/inventory` 提交配置时：
   - 若某敏感字段值为 `"********"`，服务端从磁盘现存 `configs/inventory.yaml` 读取对应原值回填，绝不将 `"********"` 写入配置文件；
   - 若 `vars.registry.password` 发生实质更新且用户未手动指定独立 `auth_basic`，服务端自动重新派生 `base64(username:new_password)`；
   - 落盘前先将原文件备份至 `configs/inventory.yaml.bak`（`0600`），再通过同目录临时文件 `rename` 原子替换。

---

## 5. 任务单飞互斥锁与 SSE 断点回放协议

### 5.1 读写任务并发分级

| 任务类别 | 代表操作 | 互斥策略 |
| :--- | :--- | :--- |
| **变更型任务（Mutating）** | `artifacts/download`、`registry/pull`、`registry/push`、`harbor/mirror/import`、`operations/prep-hosts`、`operations/run`（`deploy` 完整或 `preflight`、`cleanup`、`download`） | 全局单飞排他锁：同一 `Workspace` 任一时刻最多运行 **1** 个变更任务，冲突请求立即返回 `409 CONFLICT_ACTIVE_JOB` |
| **只读型任务与即时诊断（Read-Only）** | `inventory/validate`、`inventory/probe`、`templates/render`、`registry/ping`、`harbor/projects`、`harbor/mirror/plan`、`operations/run`（仅 `--tags check`）、`pki/verify` | 无排他写锁，允许在变更任务运行期间并发查询或预检 |

### 5.2 JSONL 事件持久化与 SSE 流格式（`GET /api/v1/jobs/{id}/events`）

每个异步任务启动后，事件同步追加写入 `<work_dir>/logs/jobs/<job_id>.jsonl`（每行一个带单调递增序号 `seq` 与 UTC 毫秒时间戳 `ts_ms` 的 JSON 对象）。客户端订阅 `GET /api/v1/jobs/{id}/events`（可选 `?after_seq=<n>` 或 `Last-Event-ID` 头）时：

1. **历史无缝回放**：先按序读取 JSONL 文件中 `seq > after_seq` 的全部历史事件逐帧下发；
2. **实时增量尾随**：若任务仍处于 `running` 状态，无缝切换至内存 `tokio::sync::broadcast` 通道按 `seq` 去重推送实时事件，每 `15s` 发送 `: keep-alive` 注释帧防反向代理空闲断连；
3. **终止收敛帧**：任务进入终态（`succeeded` / `failed` / `cancelled`）后发送最后一帧 `event: job_state` 并优雅闭合流。

```text
id: 1
event: execution
data: {"seq":1,"ts_ms":1795342400120,"event":{"kind":"play_started","play_name":"Play 1 - Preflight","hosts":["c23-p61"]}}

id: 2
event: execution
data: {"seq":2,"ts_ms":1795342400450,"event":{"kind":"task_finished","task_name":"Verify OS","host":"c23-p61","changed":false,"failed":false,"skipped":false,"stdout":"debian-13","stderr":"","duration_ms":312}}

id: 3
event: job_state
data: {"seq":3,"ts_ms":1795342400460,"status":"succeeded","exit_code":0}
```
