use std::io::Write as _;
#[derive(Clone)]
pub(crate) enum TaskStatus {
    Running {
        percent: u32,
        text: String,
        spinner_idx: usize,
        is_explicit: bool,
    },
    Success,
    Failed(String),
}
pub(crate) struct TaskProgress {
    pub host: String,
    pub task_name: String,
    pub status: TaskStatus,
    pub finished_printed: bool,
}
impl TaskProgress {
    fn render_line(&self, spinners: &[&str]) -> String {
        let bar = match &self.status {
            TaskStatus::Running { percent, .. } => make_progress_bar(*percent),
            TaskStatus::Success | TaskStatus::Failed(_) => make_progress_bar(100),
        };
        let (icon, pct_str, suffix_raw) = match &self.status {
            TaskStatus::Running {
                percent,
                text,
                spinner_idx,
                is_explicit,
            } => {
                let spinner = spinners.get(*spinner_idx).unwrap_or(&"⠋");
                let pct_display = if *is_explicit {
                    format!("{percent}%")
                } else {
                    "  ...".to_string()
                };
                (
                    *spinner,
                    pct_display,
                    if text.is_empty() {
                        String::new()
                    } else {
                        format!(" ({text})")
                    },
                )
            }
            TaskStatus::Success => ("✔", "100%".to_string(), String::new()),
            TaskStatus::Failed(err) => ("✘", "FAILED".to_string(), format!(" ({err})")),
        };
        let display_name = if self.host == "localhost" {
            self.task_name.clone()
        } else {
            format!(
                "{host}: {task_name}",
                host = self.host,
                task_name = self.task_name
            )
        };
        let term_w = terminal_width();
        let name_width = std::cmp::max(20, term_w.saturating_sub(61));
        let name_padded = format_fixed_width(&display_name, name_width);
        let pct_padded = format!("{pct_str:>6}");
        let suffix_padded = format_fixed_width(&suffix_raw, 28);
        format!("{icon} {name_padded} {bar} {pct_padded} {suffix_padded}")
    }
}
fn terminal_width() -> usize {
    if let Some((terminal_size::Width(w), _)) = terminal_size::terminal_size() {
        usize::from(w)
    } else {
        80
    }
}
fn format_fixed_width(s: &str, width: usize) -> String {
    let char_count = s.chars().count();
    if char_count > width {
        let mut truncated = s.chars().take(width - 3).collect::<String>();
        truncated.push_str("...");
        truncated
    } else {
        format!("{s:<width$}")
    }
}
fn make_progress_bar(percent: u32) -> String {
    let width = 20usize;
    let total_steps = width * 2;
    let filled_steps = (usize::try_from(percent).unwrap_or(0) * total_steps) / 100;
    let mut bar = String::new();
    for i in 0..width {
        let left_filled = 2 * i < filled_steps;
        let right_filled = 2 * i + 1 < filled_steps;
        let code = match (left_filled, right_filled) {
            (true, true) => 0xFF,
            (true, false) => 0x47,
            (false, true) => 0xB8,
            (false, false) => 0x00,
        };
        let c = char::from_u32(0x2800 + code).unwrap_or('⠀');
        bar.push(c);
    }
    format!("[{bar}]")
}
pub(crate) struct ProgressTracker {
    pub tasks: Vec<TaskProgress>,
    pub active_lines_count: usize,
}
impl ProgressTracker {
    pub(crate) fn new() -> Self {
        Self {
            tasks: Vec::new(),
            active_lines_count: 0,
        }
    }
    pub(crate) fn log_message(&mut self, msg: &str) {
        let trimmed_msg = msg.trim();
        let should_suppress = trimmed_msg.starts_with("TASK [")
            || trimmed_msg == "changed"
            || trimmed_msg.starts_with("changed:")
            || trimmed_msg == "ok"
            || trimmed_msg.starts_with("ok:")
            || trimmed_msg == "skipped"
            || trimmed_msg.starts_with("skipped:")
            || trimmed_msg.starts_with("fatal:")
            || trimmed_msg.starts_with("Uploading manifest")
            || trimmed_msg.starts_with("Successfully pushed manifest")
            || trimmed_msg.starts_with("Processing:")
            || trimmed_msg.starts_with("Uploading blob")
            || trimmed_msg.starts_with("Successfully uploaded blob")
            || trimmed_msg.starts_with("Starting pure-Rust HTTP API upload")
            || trimmed_msg.starts_with("Using registry base URL:")
            || (trimmed_msg.starts_with("Blob ") && trimmed_msg.contains("already exists"))
            || trimmed_msg.starts_with("Resolving actual OS hostnames")
            || trimmed_msg.starts_with("Updating /etc/hosts")
            || trimmed_msg.starts_with("Reading inventory")
            || trimmed_msg.starts_with("Successfully updated /etc/hosts")
            || trimmed_msg.starts_with("Prep hosts completed")
            || (trimmed_msg.starts_with("- ") && trimmed_msg.contains("-> OS hostname:"));
        if self.active_lines_count > 0 {
            print!("\x1B[{}A", self.active_lines_count);
            for _ in 0..self.active_lines_count {
                println!("\x1B[2K");
            }
            print!("\x1B[{}A", self.active_lines_count);
            let _ = std::io::stdout().flush();
        }
        if !should_suppress {
            print!("{msg}");
            let _ = std::io::stdout().flush();
        }
        let spinners = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let mut finished_to_print = Vec::new();
        for t in &mut self.tasks {
            if !t.finished_printed {
                match t.status {
                    TaskStatus::Success | TaskStatus::Failed(_) => {
                        finished_to_print.push(t);
                    }
                    TaskStatus::Running { .. } => {}
                }
            }
        }
        for t in finished_to_print {
            let line = t.render_line(&spinners);
            println!("{line}");
            t.finished_printed = true;
        }
        let running_tasks: Vec<&TaskProgress> =
            self.tasks.iter().filter(|t| !t.finished_printed).collect();
        for t in &running_tasks {
            let line = t.render_line(&spinners);
            println!("{line}");
        }
        self.active_lines_count = running_tasks.len();
        let _ = std::io::stdout().flush();
    }
    pub(crate) fn draw_active(&mut self) {
        let spinners = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let mut finished_to_print = Vec::new();
        for t in &mut self.tasks {
            if !t.finished_printed {
                match t.status {
                    TaskStatus::Success | TaskStatus::Failed(_) => {
                        finished_to_print.push(t);
                    }
                    TaskStatus::Running { .. } => {}
                }
            }
        }
        if self.active_lines_count > 0 {
            print!("\x1B[{}A", self.active_lines_count);
            for _ in 0..self.active_lines_count {
                println!("\x1B[2K");
            }
            print!("\x1B[{}A", self.active_lines_count);
            let _ = std::io::stdout().flush();
        }
        for t in finished_to_print {
            let line = t.render_line(&spinners);
            println!("{line}");
            t.finished_printed = true;
        }
        let running_tasks: Vec<&TaskProgress> =
            self.tasks.iter().filter(|t| !t.finished_printed).collect();
        for t in &running_tasks {
            let line = t.render_line(&spinners);
            println!("{line}");
        }
        self.active_lines_count = running_tasks.len();
        let _ = std::io::stdout().flush();
    }
}
