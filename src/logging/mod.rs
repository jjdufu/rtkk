use std::io::IsTerminal as _;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tracing_appender::non_blocking;
use tracing_subscriber::{Registry, prelude::*};
pub(crate) mod progress;
use progress::{ProgressTracker, TaskProgress, TaskStatus};
static WORKER_GUARD: OnceLock<non_blocking::WorkerGuard> = OnceLock::new();
static PROGRESS_TRACKER: OnceLock<Arc<Mutex<ProgressTracker>>> = OnceLock::new();
static SHUTDOWN_LOGGER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub(crate) struct Logger;
impl Logger {
    pub(crate) fn init(file_path: Option<&str>) -> Result<(), String> {
        let path_str = file_path.unwrap_or("deploy.log");
        let path = Path::new(path_str);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create log directory: {e}"))?;
            }
        }
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("deploy.log");
        let parent_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file_appender = tracing_appender::rolling::never(parent_dir, file_name);
        let (non_blocking_writer, guard) = non_blocking(file_appender);
        let file_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(non_blocking_writer)
            .with_filter(tracing_subscriber::filter::LevelFilter::INFO);
        let subscriber = Registry::default().with(file_layer);
        tracing::subscriber::set_global_default(subscriber)
            .map_err(|e| format!("failed to set tracing global default: {e}"))?;
        let _ = WORKER_GUARD.set(guard);
        if std::io::stdout().is_terminal() {
            init_progress();
        }
        Ok(())
    }
    pub(crate) fn log(msg: &str) {
        if let Some(tracker_arc) = PROGRESS_TRACKER.get() {
            if let Ok(mut tracker) = tracker_arc.lock() {
                tracker.log_message(msg);
            } else {
                if std::io::stdout().is_terminal() {
                    print!("{msg}");
                } else {
                    print!("{}", crate::cli::clean_ansi_escapes(msg));
                }
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
        } else {
            if std::io::stdout().is_terminal() {
                print!("{msg}");
            } else {
                print!("{}", crate::cli::clean_ansi_escapes(msg));
            }
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
        let cleaned = crate::cli::clean_ansi_escapes(msg);
        let trimmed = cleaned.trim_end();
        if !trimmed.is_empty() {
            tracing::info!("{}", trimmed);
        }
    }
}
fn init_progress() {
    let tracker = Arc::new(Mutex::new(ProgressTracker::new()));
    let tracker_clone = Arc::clone(&tracker);
    let _ = PROGRESS_TRACKER.set(tracker);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(80));
        let mut tick_count = 0u64;
        loop {
            if SHUTDOWN_LOGGER.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            interval.tick().await;
            tick_count += 1;
            let Ok(mut tracker_guard) = tracker_clone.lock() else {
                break;
            };
            if crate::utils::env_flag_enabled("RTKK_LEGACY_FAKE_PROGRESS") {
                for t in &mut tracker_guard.tasks {
                    if let TaskStatus::Running {
                        ref mut percent,
                        is_explicit,
                        ..
                    } = t.status
                    {
                        if !is_explicit && tick_count % 3 == 0 && *percent < 95 {
                            *percent += 1;
                        }
                    }
                }
            }
            let spinners_len = 10u64;
            let spinner_idx = usize::try_from(tick_count % spinners_len).unwrap_or(0);
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
pub(crate) fn shutdown_logging() {
    SHUTDOWN_LOGGER.store(true, std::sync::atomic::Ordering::Relaxed);
}
pub(crate) fn start_task(host: &str, task_name: &str) {
    if let Some(tracker_arc) = PROGRESS_TRACKER.get() {
        if let Ok(mut tracker) = tracker_arc.lock() {
            tracker
                .tasks
                .retain(|t| !(t.host == host && t.task_name == task_name));
            tracker.tasks.push(TaskProgress {
                host: host.to_string(),
                task_name: task_name.to_string(),
                status: TaskStatus::Running {
                    percent: 0,
                    text: String::new(),
                    spinner_idx: 0,
                    is_explicit: false,
                },
                finished_printed: false,
            });
        }
    }
}
pub(crate) fn update_task(host: &str, task_name: &str, percent: u32, text: &str) {
    if let Some(tracker_arc) = PROGRESS_TRACKER.get() {
        if let Ok(mut tracker) = tracker_arc.lock() {
            if let Some(t) = tracker
                .tasks
                .iter_mut()
                .find(|t| t.host == host && t.task_name == task_name && !t.finished_printed)
            {
                t.status = TaskStatus::Running {
                    percent,
                    text: text.to_string(),
                    spinner_idx: match t.status {
                        TaskStatus::Running { spinner_idx, .. } => spinner_idx,
                        _ => 0,
                    },
                    is_explicit: true,
                };
            }
        }
    }
}
pub(crate) fn finish_task_success(host: &str, task_name: &str) {
    if let Some(tracker_arc) = PROGRESS_TRACKER.get() {
        if let Ok(mut tracker) = tracker_arc.lock() {
            if let Some(t) = tracker
                .tasks
                .iter_mut()
                .find(|t| t.host == host && t.task_name == task_name && !t.finished_printed)
            {
                t.status = TaskStatus::Success;
            }
        }
    }
}
pub(crate) fn finish_task_failed(host: &str, task_name: &str, err: &str) {
    if let Some(tracker_arc) = PROGRESS_TRACKER.get() {
        if let Ok(mut tracker) = tracker_arc.lock() {
            if let Some(t) = tracker
                .tasks
                .iter_mut()
                .find(|t| t.host == host && t.task_name == task_name && !t.finished_printed)
            {
                t.status = TaskStatus::Failed(err.to_string());
            }
        }
    }
}
#[macro_export]
macro_rules! log_println {
    () => {
        $crate::logging::Logger::log("\n");
    };
    ($($arg:tt)*) => {
        $crate::logging::Logger::log(&format!("{}\n", format_args!($($arg)*)));
    };
}
#[macro_export]
macro_rules! log_print {
    ($($arg:tt)*) => {
        $crate::logging::Logger::log(&format!("{}", format_args!($($arg)*)));
    };
}
