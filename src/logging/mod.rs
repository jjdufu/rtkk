//! Structured file logging and interactive terminal progress tracking.

use std::io::IsTerminal as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tracing_appender::non_blocking;
use tracing_subscriber::{Registry, prelude::*};

pub(crate) mod progress;

use progress::{ProgressTracker, TaskProgress, TaskStatus};

static ACTIVE_WRITER: Mutex<Option<non_blocking::NonBlocking>> = Mutex::new(None);
static WORKER_GUARD: Mutex<Option<non_blocking::WorkerGuard>> = Mutex::new(None);
static PROGRESS_TRACKER: OnceLock<Arc<Mutex<ProgressTracker>>> = OnceLock::new();
static PROGRESS_LOOP_STARTED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_LOGGER: AtomicBool = AtomicBool::new(false);

struct DynamicLogWriter;

impl std::io::Write for DynamicLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let writer = ACTIVE_WRITER.lock().ok().and_then(|g| g.clone());
        if let Some(mut nb) = writer {
            nb.write(buf)
        } else {
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let writer = ACTIVE_WRITER.lock().ok().and_then(|g| g.clone());
        if let Some(mut nb) = writer {
            nb.flush()
        } else {
            Ok(())
        }
    }
}

/// Central logger coordinating file append output and `TTY` task progress bars.
pub struct Logger;

impl Logger {
    /// Initializes non-blocking file logging to `file_path` (defaulting to `"deploy.log"`)
    /// and starts interactive `TTY` progress rendering when standard output is a terminal.
    ///
    /// Safe to call multiple times in tests: subsequent calls rotate the active log file
    /// writer without failing if the global `tracing` subscriber is already installed.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent directory cannot be created or the log file cannot
    /// be opened for appending.
    pub fn init(file_path: Option<&str>) -> Result<(), String> {
        let path_str = file_path.unwrap_or("deploy.log");
        let path = Path::new(path_str);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create log directory {}: {e}", parent.display()))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("failed to open log file {}: {e}", path.display()))?;

        SHUTDOWN_LOGGER.store(false, Ordering::Relaxed);
        let (non_blocking_writer, guard) = non_blocking(file);
        if let Ok(mut active) = ACTIVE_WRITER.lock() {
            *active = Some(non_blocking_writer);
        }
        if let Ok(mut slot) = WORKER_GUARD.lock() {
            *slot = Some(guard);
        }

        let file_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(|| DynamicLogWriter)
            .with_filter(tracing_subscriber::filter::LevelFilter::INFO);
        let subscriber = Registry::default().with(file_layer);
        let _ = tracing::subscriber::set_global_default(subscriber);

        if std::io::stdout().is_terminal() && tokio::runtime::Handle::try_current().is_ok() {
            init_progress();
        }
        Ok(())
    }

    /// Writes `msg` to the console (preserving active `TTY` progress bars) and records a
    /// stripped plain-text entry in the log file.
    pub fn log(msg: &str) {
        let locked = PROGRESS_TRACKER.get().and_then(|arc| arc.lock().ok());
        if let Some(mut tracker) = locked {
            tracker.log_message(msg);
        } else {
            print_direct(msg);
        }
        let cleaned = crate::utils::clean_ansi_escapes(msg);
        let trimmed = cleaned.trim_end();
        if !trimmed.is_empty() {
            tracing::info!("{trimmed}");
        }
    }
}

fn print_direct(msg: &str) {
    if std::io::stdout().is_terminal() {
        print!("{msg}");
    } else {
        print!("{}", crate::utils::clean_ansi_escapes(msg));
    }
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

fn init_progress() {
    let tracker_arc =
        Arc::clone(PROGRESS_TRACKER.get_or_init(|| Arc::new(Mutex::new(ProgressTracker::new()))));
    if PROGRESS_LOOP_STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(80));
        let mut tick_count = 0_u64;
        loop {
            if SHUTDOWN_LOGGER.load(Ordering::Relaxed) {
                PROGRESS_LOOP_STARTED.store(false, Ordering::Relaxed);
                break;
            }
            interval.tick().await;
            tick_count = tick_count.wrapping_add(1);
            let Ok(mut tracker_guard) = tracker_arc.lock() else {
                PROGRESS_LOOP_STARTED.store(false, Ordering::Relaxed);
                break;
            };
            let spinner_idx = usize::try_from(tick_count % 10).unwrap_or(0);
            for t in &mut tracker_guard.tasks {
                if let TaskStatus::Running {
                    spinner_idx: ref mut s,
                    ..
                } = t.status
                {
                    *s = spinner_idx;
                }
            }
            tracker_guard.draw_active();
        }
    });
}

/// Stops the background progress spinner task and flushes all buffered log records to disk.
pub fn shutdown_logging() {
    SHUTDOWN_LOGGER.store(true, Ordering::Relaxed);
    if let Ok(mut active) = ACTIVE_WRITER.lock() {
        drop(active.take());
    }
    if let Ok(mut guard) = WORKER_GUARD.lock() {
        drop(guard.take());
    }
}

/// Registers a newly started task on `host` in the interactive progress tracker.
pub fn start_task(host: &str, task_name: &str) {
    let Some(tracker_arc) = PROGRESS_TRACKER.get() else {
        return;
    };
    if let Ok(mut tracker) = tracker_arc.lock() {
        tracker
            .tasks
            .retain(|t| !(t.host == host && t.task_name == task_name));
        tracker.tasks.push(TaskProgress {
            host: host.to_string(),
            task_name: task_name.to_string(),
            status: TaskStatus::Running {
                percent: None,
                text: String::new(),
                spinner_idx: 0,
            },
            finished_printed: false,
        });
    }
}

/// Updates explicit percentage completion and status `text` for a running task on `host`.
pub fn update_task(host: &str, task_name: &str, percent: u32, text: &str) {
    let Some(tracker_arc) = PROGRESS_TRACKER.get() else {
        return;
    };
    if let Ok(mut tracker) = tracker_arc.lock() {
        let found = tracker
            .tasks
            .iter_mut()
            .find(|t| t.host == host && t.task_name == task_name && !t.finished_printed);
        if let Some(t) = found {
            let spinner_idx = match t.status {
                TaskStatus::Running { spinner_idx, .. } => spinner_idx,
                TaskStatus::Success | TaskStatus::Failed(_) => 0,
            };
            t.status = TaskStatus::Running {
                percent: Some(percent.min(100)),
                text: text.to_string(),
                spinner_idx,
            };
        }
    }
}

/// Marks a running task on `host` as completed successfully.
pub fn finish_task_success(host: &str, task_name: &str) {
    let Some(tracker_arc) = PROGRESS_TRACKER.get() else {
        return;
    };
    if let Ok(mut tracker) = tracker_arc.lock() {
        let found = tracker
            .tasks
            .iter_mut()
            .find(|t| t.host == host && t.task_name == task_name && !t.finished_printed);
        if let Some(t) = found {
            t.status = TaskStatus::Success;
        }
    }
}

/// Marks a running task on `host` as failed with error summary `err`.
pub fn finish_task_failed(host: &str, task_name: &str, err: &str) {
    let Some(tracker_arc) = PROGRESS_TRACKER.get() else {
        return;
    };
    if let Ok(mut tracker) = tracker_arc.lock() {
        let found = tracker
            .tasks
            .iter_mut()
            .find(|t| t.host == host && t.task_name == task_name && !t.finished_printed);
        if let Some(t) = found {
            t.status = TaskStatus::Failed(err.to_string());
        }
    }
}

/// Formats and logs a line (with trailing newline) through [`Logger::log`].
#[macro_export]
macro_rules! log_println {
    () => {
        $crate::logging::Logger::log("\n");
    };
    ($($arg:tt)*) => {
        $crate::logging::Logger::log(&format!("{}\n", format_args!($($arg)*)));
    };
}

/// Formats and logs text (without adding a newline) through [`Logger::log`].
#[macro_export]
macro_rules! log_print {
    ($($arg:tt)*) => {
        $crate::logging::Logger::log(&format!("{}", format_args!($($arg)*)));
    };
}
