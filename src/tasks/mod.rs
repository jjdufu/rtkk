use crate::connector::Connector;
use crate::template::render_template_string;
use crate::utils::env_flag_enabled;
use serde_json::Value;
use std::collections::HashMap;
pub(crate) mod cmd;
pub(crate) mod copy;
pub(crate) mod fetch;
pub(crate) mod gen_cert;
pub(crate) mod http;
pub(crate) mod modprobe;
pub(crate) mod outcome;
pub(crate) mod remote_fs;
pub(crate) mod sysctl;
pub(crate) mod systemd;
pub(crate) mod template;
pub(crate) use outcome::TaskModuleError;
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Task {
    pub name: Option<String>,
    pub gen_cert: Option<Value>,
    pub template: Option<Value>,
    pub fetch: Option<Value>,
    pub http_get_file: Option<Value>,
    pub command: Option<Value>,
    pub shell: Option<Value>,
    pub copy: Option<Value>,
    pub systemd: Option<Value>,
    pub sysctl: Option<Value>,
    pub modprobe: Option<Value>,
    pub register: Option<String>,
    pub register_type: Option<String>,
    pub set_global: Option<bool>,
    pub when: Option<Value>,
    pub failed_when: Option<Value>,
    pub retries: Option<usize>,
    pub delay: Option<u64>,
    pub ignore_errors: Option<bool>,
    pub run_once: Option<bool>,
    pub delegate_to: Option<String>,
    pub r#loop: Option<Value>,
    pub with_items: Option<Value>,
    pub break_on_error: Option<bool>,
    pub block: Option<Vec<Task>>,
    pub rescue: Option<Vec<Task>>,
    pub always: Option<Vec<Task>>,
    pub changed_when: Option<Value>,
    pub tags: Option<Vec<String>>,
    pub timeout: Option<u64>,
    #[serde(skip)]
    pub relative_path: Option<String>,
    #[serde(skip)]
    pub role_defaults: Option<HashMap<String, Value>>,
    #[serde(skip)]
    pub role_params: Option<HashMap<String, Value>>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub(crate) struct TaskResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub changed: bool,
    pub failed: bool,
}
pub(crate) fn render_value(val: &Value, context: &gtmpl::Value) -> Result<Value, String> {
    match val {
        Value::String(s) => match render_template_string(s, context) {
            Ok(rendered) => Ok(Value::String(rendered)),
            Err(e) => {
                let has_template = s.contains("{{");
                if has_template {
                    tracing::warn!("template render failed (string contains '{{{{'): {e}");
                    if env_flag_enabled("RTKK_STRICT_TEMPLATE") {
                        return Err(format!("template render failed: {e}"));
                    }
                } else {
                    tracing::warn!("template render failed, using original string: {e}");
                }
                Ok(Value::String(s.clone()))
            }
        },
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
    ]
    .iter()
    .filter(|x| **x)
    .count()
}
pub(crate) async fn execute_task_module(
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
    if let Some(res) = http::execute_http_get_file(task, &context).await? {
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
    Err(TaskModuleError::Message(
        "unknown task module".to_string(),
    ))
}
#[must_use]
pub(crate) fn task_needs_hostvars(task: &Task) -> bool {
    value_contains_hostvars(task.when.as_ref())
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
