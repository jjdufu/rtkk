//! Playbook task definitions, template value rendering, and module dispatch.

use crate::connector::{BecomeMethod, Connector};
use crate::template::render_template_string;
use serde_json::Value;
use std::collections::HashMap;

pub(crate) mod cmd;
pub(crate) mod copy;
pub(crate) mod fetch;
pub(crate) mod gen_cert;
pub(crate) mod harbor;
pub(crate) mod http;
pub(crate) mod modprobe;
pub(crate) mod outcome;
pub(crate) mod remote_fs;
pub(crate) mod sysctl;
pub(crate) mod systemd;
pub(crate) mod template;

pub use outcome::TaskModuleError;

/// Declarative playbook task specification and execution modifiers.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Task {
    /// Human-readable display name of the task.
    pub name: Option<String>,
    /// Parameters for the `gen_cert` `TLS` certificate generation module.
    pub gen_cert: Option<Value>,
    /// Parameters for the `template` rendering module.
    pub template: Option<Value>,
    /// Parameters for the `fetch` remote-to-local file copy module.
    pub fetch: Option<Value>,
    /// Parameters for the `http_get_file` `HTTP` download module.
    pub http_get_file: Option<Value>,
    /// Parameters for the `command` direct argv execution module.
    pub command: Option<Value>,
    /// Parameters for the `shell` `/bin/sh -c` execution module.
    pub shell: Option<Value>,
    /// Parameters for the `copy` file transfer module.
    pub copy: Option<Value>,
    /// Parameters for the `systemd` unit management module.
    pub systemd: Option<Value>,
    /// Parameters for the `sysctl` kernel parameter module.
    pub sysctl: Option<Value>,
    /// Parameters for the `modprobe` kernel module loader.
    pub modprobe: Option<Value>,
    /// Parameters for the `ensure_harbor_projects` `Harbor` project bootstrapper.
    pub ensure_harbor_projects: Option<Value>,
    /// Variable name in which to store the task result.
    pub register: Option<String>,
    /// Optional format hint for parsing `stdout` when registering (`json`, `yaml`, `text`).
    pub register_type: Option<String>,
    /// Whether to export the registered variable globally across hosts.
    pub set_global: Option<bool>,
    /// Conditional expression controlling whether this task runs.
    pub when: Option<Value>,
    /// Custom failure condition evaluated against command output and exit code.
    pub failed_when: Option<Value>,
    /// Number of retry attempts upon task failure.
    pub retries: Option<usize>,
    /// Delay in seconds between retry attempts.
    pub delay: Option<u64>,
    /// Whether playbook execution continues on this host when the task fails.
    pub ignore_errors: Option<bool>,
    /// Whether to execute this task on only the first active host in the batch.
    pub run_once: Option<bool>,
    /// Optional target host override to delegate execution to.
    pub delegate_to: Option<String>,
    /// Optional privilege escalation override (`become: true` / `false`).
    #[serde(default, rename = "become")]
    pub r#become: Option<bool>,
    /// Optional privilege escalation mechanism override (`become_method: "sudo"` / `"su"`).
    #[serde(default)]
    pub become_method: Option<BecomeMethod>,
    /// Optional target account name when privilege escalation is enabled.
    #[serde(default)]
    pub become_user: Option<String>,
    /// Loop item list or expression (`loop`).
    pub r#loop: Option<Value>,
    /// Legacy loop item list alias (`with_items`).
    pub with_items: Option<Value>,
    /// Whether to abort remaining loop items on the first item error.
    pub break_on_error: Option<bool>,
    /// Grouped subtasks executed as a block.
    pub block: Option<Vec<Task>>,
    /// Recovery subtasks executed if `block` fails.
    pub rescue: Option<Vec<Task>>,
    /// Cleanup subtasks always executed after `block` / `rescue`.
    pub always: Option<Vec<Task>>,
    /// Custom condition determining whether the task is marked `changed`.
    pub changed_when: Option<Value>,
    /// Filtering tags attached to this task.
    pub tags: Option<Vec<String>>,
    /// Per-task execution timeout in seconds.
    pub timeout: Option<u64>,
    /// Role default variables bound when expanding role tasks.
    #[serde(skip)]
    pub role_defaults: Option<HashMap<String, Value>>,
    /// Role invocation parameters bound when expanding role tasks.
    #[serde(skip)]
    pub role_params: Option<HashMap<String, Value>>,
}

/// Outcome of executing a single task module on a host.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct TaskResult {
    /// Captured standard output text.
    pub stdout: String,
    /// Captured standard error text.
    pub stderr: String,
    /// Process or module exit status code (`0` indicates success).
    pub exit_code: i32,
    /// Whether target system state was modified by the task.
    pub changed: bool,
    /// Whether the task failed.
    pub failed: bool,
}

/// Recursively renders `gtmpl` template expressions (`{{ ... }}`) inside a [`serde_json::Value`].
///
/// Strings that do not contain `"{{"` are returned unchanged without invoking the template engine.
///
/// # Errors
///
/// Returns an error string if any string containing `"{{"` fails template parsing or evaluation.
pub fn render_value(val: &Value, context: &gtmpl::Value) -> Result<Value, String> {
    match val {
        Value::String(s) => {
            if !s.contains("{{") {
                return Ok(Value::String(s.clone()));
            }
            render_template_string(s, context)
                .map(Value::String)
                .map_err(|e| format!("template render failed for '{s}': {e}"))
        }
        Value::Array(arr) => {
            let mut rendered_arr = Vec::with_capacity(arr.len());
            for item in arr {
                rendered_arr.push(render_value(item, context)?);
            }
            Ok(Value::Array(rendered_arr))
        }
        Value::Object(obj) => {
            let mut rendered_obj = serde_json::Map::new();
            for (k, v) in obj {
                rendered_obj.insert(k.clone(), render_value(v, context)?);
            }
            Ok(Value::Object(rendered_obj))
        }
        _ => Ok(val.clone()),
    }
}

fn count_action_modules(task: &Task) -> usize {
    [
        task.gen_cert.is_some(),
        task.template.is_some(),
        task.fetch.is_some(),
        task.http_get_file.is_some(),
        task.command.is_some(),
        task.shell.is_some(),
        task.copy.is_some(),
        task.systemd.is_some(),
        task.sysctl.is_some(),
        task.modprobe.is_some(),
        task.ensure_harbor_projects.is_some(),
    ]
    .iter()
    .filter(|x| **x)
    .count()
}

/// Dispatches `task` to its configured action module using `connector` and `host_vars`.
///
/// # Errors
///
/// Returns [`TaskModuleError`] if zero or multiple action modules are specified, template
/// rendering fails, or module execution fails.
pub async fn execute_task_module(
    task: &Task,
    connector: &Connector,
    host_vars: &mut Value,
) -> Result<TaskResult, TaskModuleError> {
    let module_count = count_action_modules(task);
    if module_count > 1 {
        return Err(TaskModuleError::Message(
            "task specifies multiple action modules".to_string(),
        ));
    }
    if module_count == 0 && task.block.is_none() {
        return Err(TaskModuleError::Message(
            "unknown or empty task module".to_string(),
        ));
    }
    let context = crate::template::json_to_gtmpl(host_vars);
    if let Some(res) = gen_cert::execute_gen_cert(task, connector, host_vars, &context).await? {
        return Ok(res);
    }
    if let Some(res) = template::execute_template(task, connector, &context).await? {
        return Ok(res);
    }
    if let Some(res) = fetch::execute_fetch(task, connector, &context).await? {
        return Ok(res);
    }
    if let Some(res) = http::execute_http_get_file(task, connector, &context).await? {
        return Ok(res);
    }
    if let Some(res) = cmd::execute_cmd_shell(task, connector, &context).await? {
        return Ok(res);
    }
    if let Some(res) = copy::execute_copy(task, connector, &context).await? {
        return Ok(res);
    }
    if let Some(res) = systemd::execute_systemd(task, connector, &context).await? {
        return Ok(res);
    }
    if let Some(res) = sysctl::execute_sysctl(task, connector, &context).await? {
        return Ok(res);
    }
    if let Some(res) = modprobe::execute_modprobe(task, connector, &context).await? {
        return Ok(res);
    }
    if let Some(res) = harbor::execute_ensure_harbor_projects(task, host_vars, &context).await? {
        return Ok(res);
    }
    Err(TaskModuleError::Message("unknown task module".to_string()))
}

/// Returns `true` if `task` (or any nested block/rescue/always task) references `hostvars`.
#[must_use]
pub fn task_needs_hostvars(task: &Task) -> bool {
    task.delegate_to
        .as_deref()
        .is_some_and(|s| s.contains("hostvars"))
        || task
            .become_user
            .as_deref()
            .is_some_and(|s| s.contains("hostvars"))
        || value_contains_hostvars(task.when.as_ref())
        || value_contains_hostvars(task.failed_when.as_ref())
        || value_contains_hostvars(task.changed_when.as_ref())
        || value_contains_hostvars(task.r#loop.as_ref())
        || value_contains_hostvars(task.with_items.as_ref())
        || value_contains_hostvars(task.gen_cert.as_ref())
        || value_contains_hostvars(task.template.as_ref())
        || value_contains_hostvars(task.fetch.as_ref())
        || value_contains_hostvars(task.http_get_file.as_ref())
        || value_contains_hostvars(task.command.as_ref())
        || value_contains_hostvars(task.shell.as_ref())
        || value_contains_hostvars(task.copy.as_ref())
        || value_contains_hostvars(task.systemd.as_ref())
        || value_contains_hostvars(task.sysctl.as_ref())
        || value_contains_hostvars(task.modprobe.as_ref())
        || value_contains_hostvars(task.ensure_harbor_projects.as_ref())
        || task
            .block
            .as_ref()
            .is_some_and(|b| b.iter().any(task_needs_hostvars))
        || task
            .rescue
            .as_ref()
            .is_some_and(|b| b.iter().any(task_needs_hostvars))
        || task
            .always
            .as_ref()
            .is_some_and(|b| b.iter().any(task_needs_hostvars))
}

fn value_contains_hostvars(val: Option<&Value>) -> bool {
    val.is_some_and(value_has_hostvars)
}

fn value_has_hostvars(val: &Value) -> bool {
    match val {
        Value::String(s) => s.contains("hostvars"),
        Value::Array(a) => a.iter().any(value_has_hostvars),
        Value::Object(o) => o.values().any(value_has_hostvars),
        _ => false,
    }
}
