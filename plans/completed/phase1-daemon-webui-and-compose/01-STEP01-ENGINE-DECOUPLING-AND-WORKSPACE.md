# 01 · Stage B · Step 01：核心引擎去全局静态重构与 `Workspace` 抽象

本步骤在不改变现有 6 个 CLI 子命令外部行为且保持 53 项存量测试 100% 通过的前提下，消除 `src/logging/` 与 `src/template/` 的进程级单任务静态耦合，建立面向常驻 Daemon 多任务调度的结构化 `EventSink` 事件总线、实例级 `TemplateRegistry` 与自包含 `Workspace` 目录模型。

---

## 1. 目标与边界

1. **作用域化 `EventSink`**：将 Playbook 任务生命周期、镜像分层拉取/推送进度与诊断文本抽象为可序列化 `ExecutionEvent`，支持 CLI 终端进度条（`TerminalSink`）与 Daemon 任务独立广播/落盘流（`BroadcastJobSink`）无锁共存；单次任务结束不再永久关闭全局日志器。
2. **实例级 `TemplateRegistry`**：消除全局模板表并发写竞争，允许多个渲染上下文（并发 Playbook 任务与在线模板预览请求）各自持有独立模板表互不污染。
3. **自包含 `Workspace` 目录解析与首次自举**：统一管理工作区下的 `configs/`、`downloads/{bin,tar,images}`、`pki/`（`0700`）、`logs/jobs/`、`rtkk_bak/` 与 `.rtkk-daemon/`（`0700`）；当空目录首次启动缺少 `configs/inventory.yaml` 时，自动释放编译期内嵌的仓库标准 `configs/` 模板树且绝不覆盖用户已有文件。

---

## 2. 分队职责与文件变更矩阵

| 负责角色 | 动作 | 文件路径 | 变更内容说明 |
| :--- | :---: | :--- | :--- |
| `orchestrator` | 修改 | `src/lib.rs` | 导出 `pub mod workspace;` |
| `orchestrator` | 修改 | `src/error.rs` | 补充 `RtkkError::Workspace(String)` 错误变体及完整 Rustdoc |
| `orchestrator` | 修改 | `Cargo.toml` | 注册 `[[test]] name = "workspace_and_events_tests"` |
| `squad-engine` | 新建 | `src/workspace.rs` | `Workspace` 路径模型、`0700` 私密目录初始化与编译期内嵌 `configs/` 自举释放器 |
| `squad-engine` | 新建 | `src/logging/events.rs` | `ExecutionEvent` 枚举、`EventSink` Trait、`NoopSink`、`MemoryCaptureSink`、`CompositeSink` |
| `squad-engine` | 修改 | `src/logging/mod.rs` | 引入 `tokio::task_local!` 任务作用域 `EventSink` 分发，保留存量 CLI `log_println!` 兼容宏 |
| `squad-engine` | 修改 | `src/template/mod.rs` | 新增 `TemplateRegistry` 实例结构与 `render_with_registry` 方法，保留默认无状态便捷函数 |
| `squad-engine` | 修改 | `src/playbook/engine.rs` | `ExecutionEngine` 持有 `Arc<dyn EventSink>`、`TemplateRegistry` 与可选 `Workspace` 句柄，在 Play/Task 起止点发射 `ExecutionEvent` |
| `squad-engine` | 修改 | `src/registry/{mod.rs,pull.rs,client.rs}` | 分层 Blob 下载与上传循环按字节阈值发射 `ExecutionEvent::ArtifactProgress` |
| `squad-ops-qa` | 新建 | `test/workspace_and_events_tests.rs` | 双并发引擎实例事件零串扰测试、实例模板表隔离测试、`Workspace` 空目录幂等释放测试 |
| `squad-ops-qa` | 已预置 | [scripts/gate/step01.sh](../../../scripts/gate/step01.sh) | G3 门禁脚本（断言 `"concurrent-sink-isolated"`、`"workspace-bootstrap-idempotent"`） |

---

## 3. 核心 Rust 类型契约

### 3.1 结构化执行事件与接收器（`src/logging/events.rs`）

```rust
/// Structured event emitted during playbook orchestration, HTTP download, or OCI image transfer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionEvent {
    /// Emitted when a play begins against its resolved host list.
    PlayStarted {
        /// Human-readable play title.
        play_name: String,
        /// Ordered list of resolved target hostnames.
        hosts: Vec<String>,
    },
    /// Emitted immediately before a task runs on a host.
    TaskStarted {
        /// Enclosing play title.
        play_name: String,
        /// Task display name.
        task_name: String,
        /// Target hostname.
        host: String,
    },
    /// Incremental loop or transfer percentage update for an active task.
    TaskProgress {
        /// Task display name.
        task_name: String,
        /// Target hostname.
        host: String,
        /// Completion percentage (`0..=100`).
        percent: u32,
        /// Concise progress status label.
        detail: String,
    },
    /// Emitted when a task finishes (succeeded, failed, or skipped) on a host.
    TaskFinished {
        /// Task display name.
        task_name: String,
        /// Target hostname.
        host: String,
        /// Whether remote/local state changed.
        changed: bool,
        /// Whether execution failed after retries/failed_when evaluation.
        failed: bool,
        /// Whether execution was skipped by `when` or tag filter.
        skipped: bool,
        /// Sanitized captured standard output.
        stdout: String,
        /// Sanitized captured standard error.
        stderr: String,
        /// Elapsed wall-clock milliseconds.
        duration_ms: u64,
    },
    /// Byte-level transfer progress for binary downloads and OCI blobs.
    ArtifactProgress {
        /// Logical artifact or image identifier.
        item: String,
        /// Transferred byte count so far.
        bytes_done: u64,
        /// Total expected byte size if advertised by server.
        bytes_total: Option<u64>,
        /// Transfer stage (`"pull_blob"`, `"push_blob"`, `"download_file"`).
        phase: String,
    },
    /// Diagnostic log record scoped to the active execution context.
    LogLine {
        /// Severity label (`"info"`, `"warn"`, `"error"`).
        level: String,
        /// Sanitized message text without ANSI control codes.
        message: String,
    },
}

/// Thread-safe receiver for structured execution telemetry.
pub trait EventSink: Send + Sync + std::fmt::Debug {
    /// Consume a single execution event without blocking async executors.
    fn emit(&self, event: ExecutionEvent);
}
```

### 3.2 工作区标准布局与内嵌模板释放（`src/workspace.rs`）

```rust
/// Canonical directory layout rooted at a `rtkk` working directory.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: std::path::PathBuf,
}
```

| 访问方法 | 解析相对路径 | 目录权限要求 |
| :--- | :--- | :---: |
| `Workspace::root(&self)` | `<work_dir>` | `0755` |
| `Workspace::configs_dir(&self)` | `<work_dir>/configs` | `0755` |
| `Workspace::downloads_bin_dir(&self)` | `<work_dir>/downloads/bin` | `0755` |
| `Workspace::downloads_tar_dir(&self)` | `<work_dir>/downloads/tar` | `0755` |
| `Workspace::downloads_images_dir(&self)` | `<work_dir>/downloads/images` | `0755` |
| `Workspace::pki_dir(&self)` | `<work_dir>/pki` | **`0700`** |
| `Workspace::logs_jobs_dir(&self)` | `<work_dir>/logs/jobs` | `0755` |
| `Workspace::backups_dir(&self)` | `<work_dir>/rtkk_bak` | **`0700`** |
| `Workspace::daemon_state_dir(&self)` | `<work_dir>/.rtkk-daemon` | **`0700`** |

**首次自举幂等算法（`Workspace::ensure_initialized(&self) -> Result<bool, RtkkError>`）**：
1. 创建全部标准子目录，在 Unix 平台显式将 `pki/`、`rtkk_bak/`、`.rtkk-daemon/` 权限收敛为 `0o700`。
2. 内嵌静态清单包含：`configs/inventory.yaml`、`configs/images.yaml`、`configs/deploy.yaml`、`configs/download.yaml`、`configs/cleanup.yaml`、`configs/roles/**`、`configs/templates/*.gtmpl`（**严禁**内嵌本地测试文件 `configs/inventory.yaml.full`）。
3. 遍历内嵌清单：仅当目标路径在磁盘上不存在时通过临时文件原子写入；若 `configs/inventory.yaml` 已存在（包含用户修改），保持原字节不变并返回 `Ok(false)`。

---

## 4. 对抗审查要点（Verifier & RedTeam Checklist）

1. **零全局死锁与零跨任务串流**：在同一 Tokio 多线程运行时内同时 `tokio::join!` 两个 `ExecutionEngine::run_playbook`（分别注入独立 `MemoryCaptureSink` 与不同主机名），断言 `Sink A` 绝不包含 `Engine B` 的事件或日志行。
2. **零 `unsafe` 与零 `unwrap`**：`src/workspace.rs` 与 `src/logging/events.rs` 严禁使用 `as` 转换或切片下标 `[]`，全部公开项具备 `# Errors` 文档注释。
3. **敏感配置隔离**：核验内嵌资产表中不包含 `configs/inventory.yaml.full`，释放出的默认 `configs/inventory.yaml` 口令字段保持 `"change-me"`。

---

## 5. 门禁验收标准（[scripts/gate/step01.sh](../../../scripts/gate/step01.sh)）

- 追踪编号：`C15`（`"concurrent-sink-isolated"`）、`C16`（`"workspace-bootstrap-idempotent"`）。
- 存量 53 项测试与新增 `workspace_and_events_tests` 全部通过，`step01.sh` 末行输出 `GATE step01 PASS`。
