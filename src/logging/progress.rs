//! Interactive `TTY` progress bar rendering with `CJK`/fullwidth-aware width calculation.

use std::io::Write as _;

const SPINNERS: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Clone)]
pub(crate) enum TaskStatus {
    Running {
        percent: Option<u32>,
        text: String,
        spinner_idx: usize,
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
    pub(crate) fn render_line(&self, spinners: &[&str], cols: usize) -> String {
        let max_cols = cols.saturating_sub(1);
        if max_cols == 0 {
            return String::new();
        }
        let bar = match &self.status {
            TaskStatus::Running { percent, .. } => make_progress_bar(percent.unwrap_or(0)),
            TaskStatus::Success | TaskStatus::Failed(_) => make_progress_bar(100),
        };
        let (icon, pct_str, suffix_raw) = match &self.status {
            TaskStatus::Running {
                percent,
                text,
                spinner_idx,
            } => {
                let spinner = spinners.get(*spinner_idx).copied().unwrap_or("⠋");
                let pct_display = percent.map_or_else(|| "  ...".to_string(), |p| format!("{p}%"));
                let clean_text = sanitize_single_line(text);
                let suffix = if clean_text.is_empty() {
                    String::new()
                } else {
                    format!("({clean_text})")
                };
                (spinner, pct_display, suffix)
            }
            TaskStatus::Success => ("✔", "100%".to_string(), String::new()),
            TaskStatus::Failed(err) => {
                let clean_err = sanitize_single_line(err);
                let suffix = if clean_err.is_empty() {
                    String::new()
                } else {
                    format!("({clean_err})")
                };
                ("✘", "FAILED".to_string(), suffix)
            }
        };
        let raw_name = if self.host == "localhost" {
            self.task_name.clone()
        } else {
            format!("{}: {}", self.host, self.task_name)
        };
        let display_name = sanitize_single_line(&raw_name);
        let pct_padded = format!("{pct_str:>6}");

        // Fixed columns: icon(1) + ' '(1) + ' '(1) + bar(22) + ' '(1) + pct(6) = 32
        let line = if suffix_raw.is_empty() {
            let name_width = std::cmp::max(8, max_cols.saturating_sub(32));
            let name_padded = format_fixed_width(&display_name, name_width);
            format!("{icon} {name_padded} {bar} {pct_padded}")
        } else {
            let avail = max_cols.saturating_sub(33);
            let suffix_width = std::cmp::min(24, avail / 2);
            let name_width = std::cmp::max(8, avail.saturating_sub(suffix_width));
            let name_padded = format_fixed_width(&display_name, name_width);
            let suffix_trunc = truncate_to_display_width(&suffix_raw, suffix_width);
            format!("{icon} {name_padded} {bar} {pct_padded} {suffix_trunc}")
        };
        truncate_to_display_width(&line, max_cols)
    }
}

fn sanitize_single_line(s: &str) -> String {
    let stripped = crate::utils::clean_ansi_escapes(s);
    stripped
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn char_display_width(ch: char) -> usize {
    if ch.is_control() {
        return 0;
    }
    if matches!(
        ch,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    ) {
        return 0;
    }
    if matches!(
        ch,
        '\u{1100}'..='\u{115F}'
            | '\u{2E80}'..='\u{9FFF}'
            | '\u{AC00}'..='\u{D7AF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FE10}'..='\u{FE6F}'
            | '\u{FF01}'..='\u{FF60}'
            | '\u{FFE0}'..='\u{FFE6}'
            | '\u{1F300}'..='\u{1FAFF}'
            | '\u{20000}'..='\u{3FFFD}'
    ) {
        2
    } else {
        1
    }
}

/// Computes the terminal column width of `s`, accounting for wide `CJK`/fullwidth codepoints.
#[must_use]
pub(crate) fn display_width(s: &str) -> usize {
    s.chars().map(char_display_width).sum()
}

/// Truncates `s` on Unicode `char` boundaries so its visible width is at most `max_width` columns.
#[must_use]
pub(crate) fn truncate_to_display_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if display_width(s) <= max_width {
        return s.to_string();
    }
    if max_width <= 3 {
        let mut out = String::new();
        let mut used = 0_usize;
        for ch in s.chars() {
            let w = char_display_width(ch);
            if used.saturating_add(w) > max_width {
                break;
            }
            out.push(ch);
            used = used.saturating_add(w);
        }
        return out;
    }
    let target = max_width.saturating_sub(3);
    let mut out = String::new();
    let mut used = 0_usize;
    for ch in s.chars() {
        let w = char_display_width(ch);
        if used.saturating_add(w) > target {
            break;
        }
        out.push(ch);
        used = used.saturating_add(w);
    }
    out.push_str("...");
    out
}

fn terminal_width() -> usize {
    if let Some((terminal_size::Width(w), _)) = terminal_size::terminal_size() {
        let cols = usize::from(w);
        if cols > 0 { cols } else { 80 }
    } else {
        80
    }
}

fn format_fixed_width(s: &str, width: usize) -> String {
    let truncated = truncate_to_display_width(s, width);
    let current_w = display_width(&truncated);
    let pad = width.saturating_sub(current_w);
    if pad == 0 {
        truncated
    } else {
        format!("{truncated}{}", " ".repeat(pad))
    }
}

fn make_progress_bar(percent: u32) -> String {
    let width = 20_usize;
    let total_steps = width * 2;
    let clamped = percent.min(100);
    let filled_steps = (usize::try_from(clamped).unwrap_or(0) * total_steps) / 100;
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

fn should_suppress_tty_message(trimmed_msg: &str) -> bool {
    trimmed_msg.starts_with("TASK [")
        || trimmed_msg == "changed"
        || trimmed_msg.starts_with("changed:")
        || trimmed_msg == "ok"
        || trimmed_msg.starts_with("ok:")
        || trimmed_msg == "skipped"
        || trimmed_msg.starts_with("skipped:")
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
        || (trimmed_msg.starts_with("- ") && trimmed_msg.contains("-> OS hostname:"))
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

    fn clear_active_lines(&self) {
        if self.active_lines_count > 0 {
            print!("\x1B[{}A", self.active_lines_count);
            for _ in 0..self.active_lines_count {
                println!("\x1B[2K");
            }
            print!("\x1B[{}A", self.active_lines_count);
            let _ = std::io::stdout().flush();
        }
    }

    fn render_tasks(&mut self) {
        let cols = terminal_width();
        for t in &mut self.tasks {
            if !t.finished_printed
                && matches!(t.status, TaskStatus::Success | TaskStatus::Failed(_))
            {
                let line = t.render_line(&SPINNERS, cols);
                println!("{line}");
                t.finished_printed = true;
            }
        }
        let mut active_count = 0_usize;
        for t in &self.tasks {
            if !t.finished_printed {
                let line = t.render_line(&SPINNERS, cols);
                println!("{line}");
                active_count = active_count.saturating_add(1);
            }
        }
        self.active_lines_count = active_count;
        let _ = std::io::stdout().flush();
    }

    pub(crate) fn log_message(&mut self, msg: &str) {
        let should_suppress = should_suppress_tty_message(msg.trim());
        self.clear_active_lines();
        if !should_suppress {
            print!("{msg}");
            let _ = std::io::stdout().flush();
        }
        self.render_tasks();
    }

    pub(crate) fn draw_active(&mut self) {
        self.clear_active_lines();
        self.render_tasks();
    }
}
