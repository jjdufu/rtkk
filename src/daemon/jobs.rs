//! Persistent single-flight mutating [`JobManager`], cooperative cancellation, and JSONL event storage.

use crate::daemon::state::ApiError;
use crate::error::RtkkError;
use crate::logging::{EventSink, ExecutionEvent, with_event_sink};
use crate::workspace::Workspace;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Notify, broadcast};

/// Lifecycle state of an asynchronous Daemon job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Accepted and queued for execution.
    Queued,
    /// Currently running in background Tokio task.
    Running,
    /// Completed with zero unignored errors.
    Succeeded,
    /// Terminated with error.
    Failed,
    /// Aborted via cooperative cancellation request.
    Cancelled,
}

impl JobStatus {
    /// Returns `true` if this job has reached a final state.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Concurrency mutex classification for Daemon jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobConcurrencyClass {
    /// Acquires workspace-wide exclusive mutating lock (`409` on contention).
    Mutating,
    /// Runs concurrently without blocking or being blocked by mutating jobs.
    Readonly,
}

/// Per-host task outcome counters aggregated during a job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostJobStats {
    /// Succeeded unchanged tasks.
    pub ok: u32,
    /// Succeeded changed tasks.
    pub changed: u32,
    /// Failed tasks.
    pub failed: u32,
    /// Skipped tasks.
    pub skipped: u32,
}

/// Summary metadata for a running or historical job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSummary {
    /// Unique monotonic job identifier (`job_<ms>_<rand>`).
    pub id: String,
    /// Logical operation category (`"playbook_run"`, `"registry_pull"`, etc.).
    pub kind: String,
    /// Concise human-readable title.
    pub title: String,
    /// Concurrency mutex class.
    pub concurrency_class: JobConcurrencyClass,
    /// Current lifecycle status.
    pub status: JobStatus,
    /// Creation epoch timestamp in milliseconds.
    pub created_at_ms: u64,
    /// Execution start epoch timestamp in milliseconds.
    pub started_at_ms: Option<u64>,
    /// Completion epoch timestamp in milliseconds.
    pub finished_at_ms: Option<u64>,
    /// Terminal error summary if failed or cancelled.
    pub error: Option<String>,
}

/// Detailed job metadata including per-host execution statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDetail {
    /// Embedded summary fields.
    #[serde(flatten)]
    pub summary: JobSummary,
    /// Per-host outcome counters.
    pub host_stats: HashMap<String, HostJobStats>,
}

/// Single JSONL record persisted to `<work_dir>/logs/jobs/<job_id>.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequencedExecutionEvent {
    /// 1-based monotonic sequence number within this job.
    pub seq: u64,
    /// Emission UTC timestamp in milliseconds.
    pub ts_ms: u64,
    /// Underlying structured execution event.
    pub event: ExecutionEvent,
}

/// Frame broadcast over in-memory live SSE channels.
#[derive(Debug, Clone)]
pub enum LiveJobFrame {
    /// Structured execution telemetry frame.
    Execution(SequencedExecutionEvent),
    /// Terminal job state transition frame.
    State {
        /// Sequence number immediately following all execution events.
        seq: u64,
        /// UTC timestamp in milliseconds.
        ts_ms: u64,
        /// Final job status.
        status: JobStatus,
        /// Optional failure message.
        error: Option<String>,
    },
}

/// Cooperative cancellation token shared with a running job task.
#[derive(Debug, Default)]
pub struct JobCancelHandle {
    cancelled: AtomicBool,
    notify: Notify,
}

impl JobCancelHandle {
    /// Signals cancellation to all waiters.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Returns `true` if cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Resolves as soon as cancellation is requested.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

#[derive(Debug)]
struct JobRecord {
    detail: JobDetail,
    next_seq: u64,
    jsonl_path: PathBuf,
    meta_path: PathBuf,
    broadcaster: broadcast::Sender<LiveJobFrame>,
    cancel_handle: Arc<JobCancelHandle>,
}

#[derive(Debug)]
struct JobStore {
    active_mutating_job: Option<String>,
    order: Vec<String>,
    jobs: HashMap<String, JobRecord>,
}

/// Coordinates job queuing, single-flight write exclusion, JSONL persistence, and live SSE fan-out.
#[derive(Debug)]
pub struct JobManager {
    jobs_dir: PathBuf,
    store: Arc<Mutex<JobStore>>,
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn persist_meta_best_effort(meta_path: &Path, detail: &JobDetail) {
    if let Ok(bytes) = serde_json::to_vec_pretty(detail) {
        let _ = std::fs::write(meta_path, bytes);
    }
}

#[derive(Debug)]
struct JobSink {
    job_id: String,
    store: Arc<Mutex<JobStore>>,
}

impl EventSink for JobSink {
    fn emit(&self, event: ExecutionEvent) {
        let ts_ms = now_epoch_ms();
        let Ok(mut guard) = self.store.lock() else {
            return;
        };
        let Some(rec) = guard.jobs.get_mut(&self.job_id) else {
            return;
        };
        rec.next_seq = rec.next_seq.saturating_add(1);
        let seq = rec.next_seq;
        if let ExecutionEvent::TaskFinished {
            ref host,
            changed,
            failed,
            skipped,
            ..
        } = event
        {
            let entry = rec.detail.host_stats.entry(host.clone()).or_default();
            if failed {
                entry.failed = entry.failed.saturating_add(1);
            } else if skipped {
                entry.skipped = entry.skipped.saturating_add(1);
            } else if changed {
                entry.changed = entry.changed.saturating_add(1);
            } else {
                entry.ok = entry.ok.saturating_add(1);
            }
        }
        let seq_ev = SequencedExecutionEvent { seq, ts_ms, event };
        if let Ok(line) = serde_json::to_string(&seq_ev)
            && let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&rec.jsonl_path)
        {
            let _ = writeln!(file, "{line}");
        }
        let _ = rec.broadcaster.send(LiveJobFrame::Execution(seq_ev));
    }
}

impl JobManager {
    /// Initializes a [`JobManager`] backed by `<workspace>/logs/jobs`, reloading historical metadata.
    ///
    /// # Errors
    ///
    /// Returns [`RtkkError::Daemon`] if the jobs log directory cannot be created or read.
    pub fn initialize(workspace: &Workspace) -> Result<Arc<Self>, RtkkError> {
        let jobs_dir = workspace.logs_jobs_dir();
        std::fs::create_dir_all(&jobs_dir).map_err(|e| {
            RtkkError::Daemon(format!(
                "failed to create jobs directory {}: {e}",
                jobs_dir.display()
            ))
        })?;
        let mut store = JobStore {
            active_mutating_job: None,
            order: Vec::new(),
            jobs: HashMap::new(),
        };
        load_existing_jobs(&jobs_dir, &mut store);
        Ok(Arc::new(Self {
            jobs_dir,
            store: Arc::new(Mutex::new(store)),
        }))
    }

    /// Returns the currently running mutating `job_id`, if any.
    #[must_use]
    pub fn active_mutating_job_id(&self) -> Option<String> {
        self.store
            .lock()
            .ok()
            .and_then(|g| g.active_mutating_job.clone())
    }

    /// Spawns an asynchronous background job, atomically enforcing single-flight exclusion for
    /// [`JobConcurrencyClass::Mutating`] jobs.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError`] (`409 CONFLICT_ACTIVE_JOB`) if another mutating job is currently active.
    pub fn spawn_job<F, Fut>(
        self: &Arc<Self>,
        kind: impl Into<String>,
        title: impl Into<String>,
        concurrency_class: JobConcurrencyClass,
        runner: F,
    ) -> Result<JobSummary, ApiError>
    where
        F: FnOnce(Arc<JobCancelHandle>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let created_at_ms = now_epoch_ms();
        let suffix: u32 = rand::random();
        let job_id = format!("job_{created_at_ms}_{suffix:08x}");
        let jsonl_path = self.jobs_dir.join(format!("{job_id}.jsonl"));
        let meta_path = self.jobs_dir.join(format!("{job_id}.meta.json"));

        let summary = JobSummary {
            id: job_id.clone(),
            kind: kind.into(),
            title: title.into(),
            concurrency_class,
            status: JobStatus::Running,
            created_at_ms,
            started_at_ms: Some(created_at_ms),
            finished_at_ms: None,
            error: None,
        };
        let detail = JobDetail {
            summary: summary.clone(),
            host_stats: HashMap::new(),
        };
        let (tx, _rx) = broadcast::channel(1024);
        let cancel_handle = Arc::new(JobCancelHandle::default());

        {
            let mut guard = self
                .store
                .lock()
                .map_err(|_| ApiError::internal("job store lock poisoned"))?;
            if concurrency_class == JobConcurrencyClass::Mutating
                && let Some(ref active_id) = guard.active_mutating_job
            {
                return Err(ApiError::conflict_active_job(active_id));
            }
            let _ = std::fs::File::create(&jsonl_path);
            persist_meta_best_effort(&meta_path, &detail);
            if concurrency_class == JobConcurrencyClass::Mutating {
                guard.active_mutating_job = Some(job_id.clone());
            }
            guard.order.push(job_id.clone());
            guard.jobs.insert(
                job_id.clone(),
                JobRecord {
                    detail,
                    next_seq: 0,
                    jsonl_path,
                    meta_path,
                    broadcaster: tx,
                    cancel_handle: Arc::clone(&cancel_handle),
                },
            );
        }

        let mgr = Arc::clone(self);
        let sink: Arc<dyn EventSink> = Arc::new(JobSink {
            job_id: job_id.clone(),
            store: Arc::clone(&self.store),
        });
        tokio::spawn(async move {
            let cancel_for_select = Arc::clone(&cancel_handle);
            let outcome = with_event_sink(sink, async move {
                tokio::select! {
                    res = runner(Arc::clone(&cancel_for_select)) => Some(res),
                    () = cancel_for_select.cancelled() => None,
                }
            })
            .await;
            mgr.complete_job(&job_id, outcome);
        });

        Ok(summary)
    }

    fn complete_job(&self, job_id: &str, outcome: Option<Result<(), String>>) {
        let finished_ms = now_epoch_ms();
        let Ok(mut guard) = self.store.lock() else {
            return;
        };
        if guard.active_mutating_job.as_deref() == Some(job_id) {
            guard.active_mutating_job = None;
        }
        let Some(rec) = guard.jobs.get_mut(job_id) else {
            return;
        };
        if rec.detail.summary.status.is_terminal() {
            return;
        }
        let (final_status, err_msg) = match outcome {
            None => (
                JobStatus::Cancelled,
                Some("Cancelled by operator".to_string()),
            ),
            Some(Ok(())) => (JobStatus::Succeeded, None),
            Some(Err(e)) => (JobStatus::Failed, Some(e)),
        };
        rec.detail.summary.status = final_status;
        rec.detail.summary.finished_at_ms = Some(finished_ms);
        rec.detail.summary.error.clone_from(&err_msg);
        rec.next_seq = rec.next_seq.saturating_add(1);
        let state_seq = rec.next_seq;
        persist_meta_best_effort(&rec.meta_path, &rec.detail);
        let _ = rec.broadcaster.send(LiveJobFrame::State {
            seq: state_seq,
            ts_ms: finished_ms,
            status: final_status,
            error: err_msg,
        });
    }

    /// Lists jobs in reverse chronological order with optional cursor pagination.
    #[must_use]
    pub fn list_jobs(
        &self,
        limit: usize,
        cursor: Option<&str>,
    ) -> (Vec<JobSummary>, Option<String>) {
        let cap = limit.clamp(1, 100);
        let Ok(guard) = self.store.lock() else {
            return (Vec::new(), None);
        };
        let mut iter = guard.order.iter().rev();
        if let Some(cur) = cursor {
            for id in iter.by_ref() {
                if id == cur {
                    break;
                }
            }
        }
        let mut items = Vec::with_capacity(cap);
        for id in iter.by_ref() {
            if let Some(rec) = guard.jobs.get(id) {
                items.push(rec.detail.summary.clone());
                if items.len() == cap {
                    break;
                }
            }
        }
        let next_cursor = if iter.next().is_some() {
            items.last().map(|j| j.id.clone())
        } else {
            None
        };
        (items, next_cursor)
    }

    /// Retrieves detailed metadata for a single job.
    #[must_use]
    pub fn get_job(&self, job_id: &str) -> Option<JobDetail> {
        let guard = self.store.lock().ok()?;
        guard.jobs.get(job_id).map(|r| r.detail.clone())
    }

    /// Requests cooperative cancellation of a job.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::not_found`] if `job_id` does not exist.
    pub fn cancel_job(&self, job_id: &str) -> Result<JobStatus, ApiError> {
        let (handle, already_terminal, current_status) = {
            let guard = self
                .store
                .lock()
                .map_err(|_| ApiError::internal("job store lock poisoned"))?;
            let rec = guard
                .jobs
                .get(job_id)
                .ok_or_else(|| ApiError::not_found(format!("job '{job_id}' not found")))?;
            (
                Arc::clone(&rec.cancel_handle),
                rec.detail.summary.status.is_terminal(),
                rec.detail.summary.status,
            )
        };
        if already_terminal {
            return Ok(current_status);
        }
        handle.cancel();
        self.complete_job(job_id, None);
        Ok(JobStatus::Cancelled)
    }

    /// Subscribes to a job's live broadcast channel and returns `(detail, jsonl_path, receiver)`.
    ///
    /// Subscribing inside the job store lock guarantees zero lost frames between JSONL disk replay
    /// and live channel tailing.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::not_found`] if `job_id` does not exist.
    pub fn subscribe_job(
        &self,
        job_id: &str,
    ) -> Result<(JobDetail, PathBuf, broadcast::Receiver<LiveJobFrame>), ApiError> {
        let guard = self
            .store
            .lock()
            .map_err(|_| ApiError::internal("job store lock poisoned"))?;
        let rec = guard
            .jobs
            .get(job_id)
            .ok_or_else(|| ApiError::not_found(format!("job '{job_id}' not found")))?;
        Ok((
            rec.detail.clone(),
            rec.jsonl_path.clone(),
            rec.broadcaster.subscribe(),
        ))
    }

    /// Cancels any active running jobs on graceful Daemon shutdown.
    pub fn cancel_all_running(&self) {
        let handles: Vec<Arc<JobCancelHandle>> = self.store.lock().map_or_default(|g| {
            g.jobs
                .values()
                .filter(|r| !r.detail.summary.status.is_terminal())
                .map(|r| Arc::clone(&r.cancel_handle))
                .collect()
        });
        for h in handles {
            h.cancel();
        }
    }
}

fn load_existing_jobs(jobs_dir: &Path, store: &mut JobStore) {
    let Ok(entries) = std::fs::read_dir(jobs_dir) else {
        return;
    };
    let mut loaded: Vec<JobDetail> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".meta.json"))
        {
            continue;
        }
        let Ok(raw) = std::fs::read(&path) else {
            continue;
        };
        let Ok(mut detail) = serde_json::from_slice::<JobDetail>(&raw) else {
            continue;
        };
        if !detail.summary.status.is_terminal() {
            detail.summary.status = JobStatus::Failed;
            detail.summary.error = Some("Interrupted by daemon restart".to_string());
            persist_meta_best_effort(&path, &detail);
        }
        loaded.push(detail);
    }
    loaded.sort_by_key(|d| d.summary.created_at_ms);
    for detail in loaded {
        let id = detail.summary.id.clone();
        let jsonl_path = jobs_dir.join(format!("{id}.jsonl"));
        let meta_path = jobs_dir.join(format!("{id}.meta.json"));
        let (tx, _) = broadcast::channel(16);
        store.order.push(id.clone());
        store.jobs.insert(
            id,
            JobRecord {
                detail,
                next_seq: 0,
                jsonl_path,
                meta_path,
                broadcaster: tx,
                cancel_handle: Arc::new(JobCancelHandle::default()),
            },
        );
    }
}
