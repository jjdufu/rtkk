//! Structured execution events and pluggable [`EventSink`] implementations.

use std::sync::{Arc, Mutex};

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
        /// Whether execution failed after retries or `failed_when` evaluation.
        failed: bool,
        /// Whether execution was skipped by `when` or `run_once` host filtering.
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
        /// Total expected byte size if advertised.
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

/// Thread-safe receiver for structured [`ExecutionEvent`] telemetry.
pub trait EventSink: Send + Sync + std::fmt::Debug {
    /// Consumes a single execution event without blocking async executors.
    fn emit(&self, event: ExecutionEvent);
}

/// Discards all emitted events.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _event: ExecutionEvent) {}
}

/// Captures emitted [`ExecutionEvent`] records in memory for tests and isolated inspections.
#[derive(Debug, Default, Clone)]
pub struct MemoryCaptureSink {
    events: Arc<Mutex<Vec<ExecutionEvent>>>,
}

impl MemoryCaptureSink {
    /// Creates an empty [`MemoryCaptureSink`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot of all events captured so far in emission order.
    #[must_use]
    pub fn snapshot(&self) -> Vec<ExecutionEvent> {
        self.events
            .lock()
            .map_or_else(|_| Vec::new(), |guard| guard.clone())
    }
}

impl EventSink for MemoryCaptureSink {
    fn emit(&self, event: ExecutionEvent) {
        if let Ok(mut guard) = self.events.lock() {
            guard.push(event);
        }
    }
}

/// Fan-out [`EventSink`] forwarding each event to multiple child sinks in order.
#[derive(Debug, Default, Clone)]
pub struct CompositeSink {
    sinks: Vec<Arc<dyn EventSink>>,
}

impl CompositeSink {
    /// Creates a [`CompositeSink`] over `sinks`.
    #[must_use]
    pub fn new(sinks: Vec<Arc<dyn EventSink>>) -> Self {
        Self { sinks }
    }
}

impl EventSink for CompositeSink {
    fn emit(&self, event: ExecutionEvent) {
        for sink in &self.sinks {
            sink.emit(event.clone());
        }
    }
}
