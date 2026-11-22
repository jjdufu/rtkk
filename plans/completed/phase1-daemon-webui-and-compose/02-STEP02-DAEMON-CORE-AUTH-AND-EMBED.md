# 02 · Stage B · Step 02：CLI / Daemon 双入口核心（`:19912`）、令牌鉴权、SSE 任务流与静态内嵌

本步骤构建 `rtkk serve` 常驻 HTTP/SSE 服务底座与 `rtkk healthcheck` 探活子命令，落地高熵令牌与 `HttpOnly` 会话双通道鉴权、工作区写任务全局单飞互斥锁（`JobManager`）、断点可回放的 JSONL + SSE 事件流，以及编译期 `build.rs` 静态资源内嵌引擎。

---

## 1. 目标与边界

1. **双入口 CLI 扩展**：在 `src/cli.rs` 新增 `Serve`（默认 `127.0.0.1:19912`）与 `Healthcheck`（默认 `http://127.0.0.1:19912/api/v1/health`）子命令，支持命令行参数与 `RTKK_*` 环境变量双重配置。
2. **高熵令牌与会话鉴权**：首次启动未指定令牌时自动以 CSPRNG 生成 32 字节（43 字符 Base64URL）令牌并以 `0600` 写入 `.rtkk-daemon/admin.token`；使用 `subtle::ConstantTimeEq` 校验 Bearer 头与登录口令，签发 `HttpOnly; SameSite=Strict` 会话 Cookie 并内置失败退避限流。
3. **持久化单飞 `JobManager` 与 SSE 回放**：变更型任务执行全局单飞互斥（冲突返回 `409 CONFLICT_ACTIVE_JOB`），只读任务并发放行；所有任务事件实时落盘至 `logs/jobs/<job_id>.jsonl`，`GET /api/v1/jobs/{id}/events` 先完整回放历史帧再无缝切换实时广播流。
4. **零外部宏 `build.rs` 静态内嵌**：当 `web/dist/` 存在时预生成 Gzip 字节、SHA256 `ETag` 与 MIME 映射表编译进二进制；当 `web/dist/` 尚未构建时生成极简离线就绪占位 HTML，保障纯后端编译与测试零阻塞。

---

## 2. 分队职责与文件变更矩阵

| 负责角色 | 动作 | 文件路径 | 变更内容说明 |
| :--- | :---: | :--- | :--- |
| `orchestrator` | 修改 | `Cargo.toml` | 按 [A6](../../../docs/standards/A6-DEPENDENCY-BASELINE.md) 引入 `axum 0.8`、`tower-http 0.6`、`subtle 2.6`、`flate2 1.1`（并在 `[build-dependencies]` 引入 `flate2`、`sha2`） |
| `orchestrator` | 新建 | `build.rs` | 扫描 `web/dist/` 输出 `$OUT_DIR/embedded_web_assets.rs`（含 Gzip 字节数组、SHA256 ETag、MIME 表与占位回退页） |
| `orchestrator` | 修改 | `src/{lib.rs,cli.rs}` | 导出 `pub mod daemon;`，新增 `Commands::Serve` 与 `Commands::Healthcheck` |
| `orchestrator` | 新建 | `src/daemon/{mod.rs,state.rs,routes/mod.rs}` | `DaemonConfig`、`DaemonState`、`ApiError` 统一三字段 JSON 响应、`X-Request-Id` 中间件、优雅停机与 Step 02 路由挂载 |
| `orchestrator` | 修改 | [A4-API-CONTRACT-INDEX.md](../../../docs/standards/A4-API-CONTRACT-INDEX.md) | 将 Step 02 的 9 条路由从「待挂载表」迁入「已挂载表」 |
| `squad-daemon` | 新建 | `src/daemon/auth.rs` | 令牌自举、`ConstantTimeEq` 比对、内存会话表、源 IP 失败退避限流与 `RequireAuth` 提取器 |
| `squad-daemon` | 新建 | `src/daemon/jobs.rs` | `JobManager`、单飞写锁、`CancellationToken` 协作取消、JSONL 追加写盘与任务元数据恢复 |
| `squad-daemon` | 新建 | `src/daemon/sse.rs` | JSONL 历史帧读取 + `broadcast::Receiver` 去重尾随 + `15s` Keep-Alive SSE 构造器 |
| `squad-daemon` | 新建 | `src/daemon/embed.rs` | 内嵌静态资产分发、`Accept-Encoding: gzip`、`If-None-Match` `304`、SPA `index.html` 回退与 `/api/*` `404` JSON 隔离 |
| `squad-daemon` | 新建 | `src/daemon/routes/{health.rs,auth.rs,jobs.rs}` | 实现本步 9 条核心接口处理器 |
| `squad-ops-qa` | 新建 | `test/daemon_core_tests.rs` | 路由鉴权、令牌 `0600` 权限、单飞锁竞争、SSE 历史+实时无损回放集成测试 |
| `squad-ops-qa` | 已预置 | [scripts/gate/step02.sh](../../../scripts/gate/step02.sh) | G3 门禁脚本（覆盖 `C17`、`C18`、`C19`） |

---

## 3. CLI 与 HTTP 接口详细契约

### 3.1 CLI 子命令参数契约

```text
rtkk serve [--listen <ADDR>] [--work-dir <DIR>] [--token-file <PATH>]
  --listen      监听地址，默认读取 RTKK_BIND_ADDR 或 127.0.0.1:19912
  --work-dir    工作区根目录，默认读取 RTKK_WORK_DIR 或 "."
  --token-file  管理令牌文件路径，默认读取 RTKK_TOKEN_FILE 或 <work-dir>/.rtkk-daemon/admin.token

rtkk healthcheck [--url <URL>] [--timeout-secs <N>]
  --url           探活完整地址，默认 http://127.0.0.1:19912/api/v1/health
  --timeout-secs  单次请求超时秒数，默认 3；HTTP 200 且 status=="ok" 返回退出码 0，否则返回 1
```

### 3.2 本步挂载的 9 条接口请求/响应模型

| 方法与路径 | 鉴权 | 请求参数 / Body | 成功响应（`200 OK`） |
| :--- | :---: | :--- | :--- |
| `GET /api/v1/health` | 公开 | 无 | `{"status":"ok","version":"0.0.5","uptime_secs":12}` |
| `POST /api/v1/auth/login` | 公开 | `{"token":"<string>"}` | `{"authenticated":true,"expires_in_secs":43200}` + `Set-Cookie: rtkk_session=...` |
| `POST /api/v1/auth/logout` | 已登录 | 无 | `{"authenticated":false}` + 清除 Cookie |
| `GET /api/v1/auth/me` | 已登录 | 无 | `{"authenticated":true,"version":"0.0.5","work_dir":"/abs/path","bind_addr":"127.0.0.1:19912"}` |
| `GET /api/v1/system/overview` | 已登录 | 无 | `{"workspace_initialized":true,"active_mutating_job_id":null,"recent_jobs":[],"counts":{"hosts":6,"binaries_ready":0,"binaries_total":5,"images_ready":0}}` |
| `GET /api/v1/jobs` | 已登录 | `?limit=20&cursor=<id>` | `{"items":[JobSummary],"next_cursor":null}` |
| `GET /api/v1/jobs/{id}` | 已登录 | 路径参数 `id` | `JobDetail`（含 `id`、`kind`、`concurrency_class`、`status`、`created_at_ms`、`started_at_ms`、`finished_at_ms`、`error`、`host_stats`） |
| `POST /api/v1/jobs/{id}/cancel` | 已登录 | 路径参数 `id` | `{"id":"job_...","status":"cancelled"}` |
| `GET /api/v1/jobs/{id}/events` | 已登录 | `?after_seq=0` / `Last-Event-ID` | `text/event-stream`（帧 `event: execution` + 终态帧 `event: job_state`） |

---

## 4. 对抗审查要点（Verifier & RedTeam Checklist）

1. **时序侧信道与空令牌防御**：拒绝长度 `< 16` 字节的自定义 `RTKK_AUTH_TOKEN` 启动；比对令牌前先归一化为 SHA256 定长 32 字节摘要再执行 `ConstantTimeEq::ct_eq`，消除令牌长度侧信道。
2. **`/api/*` 与 SPA Fallback 严格边界**：访问未知路径 `GET /api/v1/does-not-exist` 必须返回 `404` `application/json`（含 `code: "NOT_FOUND"` 与 `request_id`），绝不能回退返回 `200` `index.html`。
3. **TOCTOU 单飞互斥锁**：`JobManager::spawn_mutating` 必须在同一同步锁临界区内完成「检查当前活跃写任务为空 → 登记新活跃 `job_id`」，杜绝高并发双请求同时穿过检查的竞态窗口。
4. **优雅停机刷盘**：收到 `SIGTERM` 或 `SIGINT` 时，先触发活跃任务取消令牌、刷空打开的 `.jsonl` 文件句柄，再退出监听循环。

---

## 5. 门禁验收标准（[scripts/gate/step02.sh](../../../scripts/gate/step02.sh)）

- `python3 scripts/check_routes.py` 双向核验 9 条挂载路由 100% 对齐。
- 追踪编号：`C17`（`"daemon-healthcheck-ok"`）、`C18`（`"unauth-401-blocked"`、`"login-session-ok"`）、`C19`（`"mutating-job-mutex"`、`"sse-replay-complete"`）。
- `step02.sh` 末行输出 `GATE step02 PASS`。
