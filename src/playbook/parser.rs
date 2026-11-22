//! Playbook `YAML` loader, role compiler, role variable propagator, and `.gtmpl` auto-registrar.

use crate::playbook::{Play, Playbook};
use crate::tasks::Task;
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

fn is_yaml_path(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
}

fn is_gtmpl_path(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gtmpl"))
}

pub(crate) fn register_gtmpl_directory(templates_dir: &Path) -> Result<(), String> {
    if !templates_dir.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(templates_dir).map_err(|e| {
        format!(
            "failed to read templates directory {}: {e}",
            templates_dir.display()
        )
    })?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| is_gtmpl_path(p))
        .collect();
    paths.sort();
    for path in paths {
        let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read template {}: {e}", path.display()))?;
        crate::template::register_template(file_name, &content)?;
    }
    Ok(())
}

pub(crate) fn load_role_defaults(base_dir: &Path, role_name: &str) -> HashMap<String, Value> {
    let mut defaults = HashMap::new();
    let role_dir = base_dir.join("roles").join(role_name);
    let defaults_dir = role_dir.join("defaults");
    let main_yaml_path = defaults_dir.join("main.yaml");
    let main_yml_path = defaults_dir.join("main.yml");
    let files_to_read = if main_yaml_path.exists() {
        vec![main_yaml_path]
    } else if main_yml_path.exists() {
        vec![main_yml_path]
    } else {
        let main_dir = defaults_dir.join("main");
        if main_dir.is_dir()
            && let Ok(entries) = std::fs::read_dir(&main_dir)
        {
            let mut paths: Vec<PathBuf> = entries
                .filter_map(std::result::Result::ok)
                .map(|e| e.path())
                .filter(|p| is_yaml_path(p))
                .collect();
            paths.sort();
            paths
        } else {
            Vec::new()
        }
    };
    for path in files_to_read {
        if let Ok(content) = std::fs::read_to_string(&path)
            && let Ok(Value::Object(obj)) = serde_yml::from_str::<Value>(&content)
        {
            for (k, v) in obj {
                defaults.insert(k, v);
            }
        }
    }
    defaults
}

fn propagate_role_vars_recursive(
    tasks: &mut [Task],
    defaults: &HashMap<String, Value>,
    params: &HashMap<String, Value>,
    role_tags: Option<&[String]>,
) {
    for task in tasks {
        if !defaults.is_empty() {
            let entry = task.role_defaults.get_or_insert_with(HashMap::new);
            for (k, v) in defaults {
                entry.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        if !params.is_empty() {
            let entry = task.role_params.get_or_insert_with(HashMap::new);
            for (k, v) in params {
                entry.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        if let Some(rtags) = role_tags
            && !rtags.is_empty()
        {
            let existing = task.tags.get_or_insert_with(Vec::new);
            for tag in rtags {
                if !existing.iter().any(|t| t == tag) {
                    existing.push(tag.clone());
                }
            }
        }
        let child_defaults = task.role_defaults.clone().unwrap_or_default();
        let child_params = task.role_params.clone().unwrap_or_default();
        if let Some(ref mut b) = task.block {
            propagate_role_vars_recursive(b, &child_defaults, &child_params, role_tags);
        }
        if let Some(ref mut r) = task.rescue {
            propagate_role_vars_recursive(r, &child_defaults, &child_params, role_tags);
        }
        if let Some(ref mut a) = task.always {
            propagate_role_vars_recursive(a, &child_defaults, &child_params, role_tags);
        }
    }
}

fn extract_role_tags(val: &Value) -> Option<Vec<String>> {
    match val {
        Value::String(s) if !s.trim().is_empty() => Some(vec![s.trim().to_string()]),
        Value::Array(arr) => {
            let out: Vec<String> = arr
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if out.is_empty() { None } else { Some(out) }
        }
        _ => None,
    }
}

fn compile_single_role(base_dir: &Path, role_val: &Value) -> Result<Vec<Task>, String> {
    let role_name = if let Some(s) = role_val.as_str() {
        s.to_string()
    } else if let Some(obj) = role_val.as_object() {
        obj.get("role")
            .and_then(Value::as_str)
            .ok_or("role object missing 'role' name")?
            .to_string()
    } else {
        return Err("invalid role specifier".to_string());
    };
    let role_dir = base_dir.join("roles").join(&role_name);
    register_gtmpl_directory(&role_dir.join("templates"))?;
    let primary_tasks_path = role_dir.join("tasks/main.yaml");
    let fallback_tasks_path = role_dir.join("tasks/main.yml");
    let active_tasks_path = if primary_tasks_path.exists() {
        primary_tasks_path
    } else {
        fallback_tasks_path
    };
    let role_content = std::fs::read_to_string(&active_tasks_path).map_err(|e| {
        format!(
            "failed to read role tasks {}: {e}",
            active_tasks_path.display()
        )
    })?;
    let mut role_tasks: Vec<Task> = serde_yml::from_str(&role_content)
        .map_err(|e| format!("failed to parse role tasks for '{role_name}': {e}"))?;
    let defaults = load_role_defaults(base_dir, &role_name);
    let mut role_params = HashMap::new();
    let mut role_when: Option<Value> = None;
    let mut role_tags: Option<Vec<String>> = None;
    if let Some(obj) = role_val.as_object() {
        for (k, v) in obj {
            match k.as_str() {
                "role" => {}
                "when" => role_when = Some(v.clone()),
                "tags" => role_tags = extract_role_tags(v),
                _ => {
                    role_params.insert(k.clone(), v.clone());
                }
            }
        }
    }
    propagate_role_vars_recursive(
        &mut role_tasks,
        &defaults,
        &role_params,
        role_tags.as_deref(),
    );
    if let Some(when_cond) = role_when {
        let wrapper = Task {
            name: Some(format!("Role: {role_name}")),
            when: Some(when_cond),
            tags: role_tags,
            block: Some(role_tasks),
            role_defaults: if defaults.is_empty() {
                None
            } else {
                Some(defaults)
            },
            role_params: if role_params.is_empty() {
                None
            } else {
                Some(role_params)
            },
            ..Task::default()
        };
        Ok(vec![wrapper])
    } else {
        Ok(role_tasks)
    }
}

/// Parses and compiles a `YAML` playbook at `playbook_path`, expanding roles (before `play.tasks`),
/// propagating role defaults/parameters into nested blocks, and registering `templates/*.gtmpl`.
///
/// # Errors
///
/// Returns an error if the playbook or any referenced role/template file cannot be read or
/// contains invalid `YAML`.
pub fn compile_playbook(playbook_path: &Path) -> Result<Playbook, String> {
    let content = std::fs::read_to_string(playbook_path)
        .map_err(|e| format!("failed to read playbook {}: {e}", playbook_path.display()))?;
    let mut raw_playbook: Vec<Play> =
        serde_yml::from_str(&content).map_err(|e| format!("failed to parse playbook: {e}"))?;
    let base_dir = playbook_path.parent().unwrap_or_else(|| Path::new("."));
    let empty_map = HashMap::new();
    for play in &mut raw_playbook {
        let mut role_tasks = Vec::new();
        if let Some(ref roles_list) = play.roles {
            for role_val in roles_list {
                let compiled_role = compile_single_role(base_dir, role_val)?;
                role_tasks.extend(compiled_role);
            }
        }
        let mut regular_tasks = play.tasks.clone().unwrap_or_default();
        propagate_role_vars_recursive(&mut regular_tasks, &empty_map, &empty_map, None);
        let mut all_tasks = role_tasks;
        all_tasks.extend(regular_tasks);
        play.tasks = Some(all_tasks);
    }
    register_gtmpl_directory(&base_dir.join("templates"))?;
    Ok(Playbook(raw_playbook))
}

/// Static task outline item returned by `GET /api/v1/playbooks/{name}/outline`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskOutline {
    /// Human-readable task name.
    pub name: String,
    /// Tags attached to this task.
    pub tags: Vec<String>,
}

/// Static play outline item returned inside [`PlaybookOutline`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlayOutline {
    /// Play display title.
    pub name: String,
    /// Host group selector expression.
    pub hosts: String,
    /// Expanded flat list of tasks within this play.
    pub tasks: Vec<TaskOutline>,
}

/// Static structure summary of a compiled playbook.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaybookOutline {
    /// Logical playbook identifier (`"deploy"`, `"cleanup"`, `"download"`).
    pub name: String,
    /// Deduplicated sorted list of all tags present in the playbook.
    pub all_tags: Vec<String>,
    /// Ordered plays in the playbook.
    pub plays: Vec<PlayOutline>,
}

fn collect_task_outlines(
    tasks: &[Task],
    out: &mut Vec<TaskOutline>,
    tag_set: &mut std::collections::BTreeSet<String>,
) {
    for t in tasks {
        let tags = t.tags.clone().unwrap_or_default();
        for tag in &tags {
            tag_set.insert(tag.clone());
        }
        if let Some(ref b) = t.block {
            collect_task_outlines(b, out, tag_set);
        } else {
            out.push(TaskOutline {
                name: t.name.clone().unwrap_or_else(|| "unnamed task".to_string()),
                tags,
            });
        }
        if let Some(ref r) = t.rescue {
            collect_task_outlines(r, out, tag_set);
        }
        if let Some(ref a) = t.always {
            collect_task_outlines(a, out, tag_set);
        }
    }
}

/// Compiles `playbook_path` and extracts its static [`PlaybookOutline`].
///
/// # Errors
///
/// Returns an error if compiling the playbook or expanding its roles fails.
pub fn extract_playbook_outline(
    logical_name: &str,
    playbook_path: &Path,
) -> Result<PlaybookOutline, String> {
    let pb = compile_playbook(playbook_path)?;
    let mut tag_set = std::collections::BTreeSet::new();
    let mut plays = Vec::with_capacity(pb.0.len());
    for play in &pb.0 {
        let mut task_outlines = Vec::new();
        if let Some(ref tasks) = play.tasks {
            collect_task_outlines(tasks, &mut task_outlines, &mut tag_set);
        }
        plays.push(PlayOutline {
            name: play
                .name
                .clone()
                .unwrap_or_else(|| "unnamed play".to_string()),
            hosts: play.hosts.clone(),
            tasks: task_outlines,
        });
    }
    Ok(PlaybookOutline {
        name: logical_name.to_string(),
        all_tags: tag_set.into_iter().collect(),
        plays,
    })
}
