use crate::playbook::{Play, Playbook};
use crate::tasks::Task;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
        if main_dir.exists() && main_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&main_dir) {
                let mut paths: Vec<PathBuf> = entries
                    .filter_map(std::result::Result::ok)
                    .map(|e| e.path())
                    .filter(|p| {
                        let ext = p.extension().and_then(|ex| ex.to_str()).unwrap_or("");
                        ext == "yaml" || ext == "yml"
                    })
                    .collect();
                paths.sort();
                paths
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    };
    for path in files_to_read {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(Value::Object(obj)) = serde_yml::from_str::<Value>(&content) {
                for (k, v) in obj {
                    defaults.insert(k, v);
                }
            }
        }
    }
    defaults
}
pub(crate) fn compile_tasks(
    tasks: &[Task],
    relative_path: Option<&str>,
) -> Result<Vec<Task>, String> {
    let mut compiled = Vec::new();
    for task in tasks {
        let mut t = task.clone();
        t.relative_path = relative_path.map(String::from);
        if let Some(ref b) = t.block {
            t.block = Some(compile_tasks(b, relative_path)?);
        }
        if let Some(ref r) = t.rescue {
            t.rescue = Some(compile_tasks(r, relative_path)?);
        }
        if let Some(ref a) = t.always {
            t.always = Some(compile_tasks(a, relative_path)?);
        }
        compiled.push(t);
    }
    Ok(compiled)
}
fn empty_task_shell(name: Option<String>) -> Task {
    Task {
        name,
        gen_cert: None,
        template: None,
        fetch: None,
        http_get_file: None,
        command: None,
        shell: None,
        copy: None,
        systemd: None,
        sysctl: None,
        modprobe: None,
        register: None,
        register_type: None,
        set_global: None,
        when: None,
        failed_when: None,
        retries: None,
        delay: None,
        ignore_errors: None,
        run_once: None,
        delegate_to: None,
        r#loop: None,
        with_items: None,
        break_on_error: None,
        block: None,
        rescue: None,
        always: None,
        changed_when: None,
        tags: None,
        timeout: None,
        relative_path: None,
        role_defaults: None,
        role_params: None,
    }
}
pub(crate) fn compile_playbook(playbook_path: &Path) -> Result<Playbook, String> {
    let content = std::fs::read_to_string(playbook_path)
        .map_err(|e| format!("failed to read playbook {}: {e}", playbook_path.display()))?;
    let mut raw_playbook: Vec<Play> =
        serde_yml::from_str(&content).map_err(|e| format!("failed to parse playbook: {e}"))?;
    let base_dir = playbook_path.parent().unwrap_or_else(|| Path::new("."));
    for play in &mut raw_playbook {
        let mut play_tasks = play.tasks.clone().unwrap_or_default();
        if let Some(ref roles_list) = play.roles {
            for role_val in roles_list {
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
                let role_rel_path = format!("roles/{role_name}");
                let role_tasks_path = role_dir.join("tasks/main.yaml");
                let role_tasks_path_yml = role_dir.join("tasks/main.yml");
                let active_tasks_path = if role_tasks_path.exists() {
                    role_tasks_path
                } else {
                    role_tasks_path_yml
                };
                let role_content = std::fs::read_to_string(&active_tasks_path).map_err(|e| {
                    format!(
                        "failed to read role tasks {}: {e}",
                        active_tasks_path.display()
                    )
                })?;
                let role_raw_tasks: Vec<Task> = serde_yml::from_str(&role_content)
                    .map_err(|e| format!("failed to parse role tasks: {e}"))?;
                let role_compiled_tasks = compile_tasks(&role_raw_tasks, Some(&role_rel_path))?;
                let defaults = load_role_defaults(base_dir, &role_name);
                let mut role_params = HashMap::new();
                if let Some(obj) = role_val.as_object() {
                    for (k, v) in obj {
                        if k != "role" {
                            role_params.insert(k.clone(), v.clone());
                        }
                    }
                }
                let mut role_block_task = empty_task_shell(Some(format!("Role: {role_name}")));
                role_block_task.block = Some(role_compiled_tasks);
                role_block_task.relative_path = Some(role_rel_path);
                role_block_task.role_defaults = Some(defaults);
                if !role_params.is_empty() {
                    role_block_task.role_params = Some(role_params);
                }
                play_tasks.push(role_block_task);
            }
        }
        let compiled_play_tasks = compile_tasks(&play_tasks, None)?;
        play.tasks = Some(compiled_play_tasks);
    }
    Ok(Playbook(raw_playbook))
}
