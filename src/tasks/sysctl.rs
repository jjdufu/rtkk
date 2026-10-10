use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::Connector;
use crate::utils::{sed_replacement_escape, shell_single_quote};
use serde_json::Value;
fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "1".to_string() } else { "0".to_string() }),
        _ => None,
    }
}
pub(crate) async fn execute_sysctl(
    task: &Task,
    connector: &Connector,
    context: &gtmpl::Value,
) -> Result<Option<TaskResult>, TaskModuleError> {
    let Some(ref sysctl_config) = task.sysctl else {
        return Ok(None);
    };
    let rendered = render_value(sysctl_config, context)?;
    let name = rendered
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("sysctl: missing name".to_string()))?;
    let value = rendered
        .get("value")
        .and_then(value_to_string)
        .ok_or_else(|| TaskModuleError::Message("sysctl: missing value".to_string()))?;
    let sysctl_file = rendered
        .get("sysctl_file")
        .and_then(Value::as_str)
        .unwrap_or("/etc/sysctl.conf");
    let name_q = shell_single_quote(name);
    let file_q = shell_single_quote(sysctl_file);
    let value_sed = sed_replacement_escape(&value);
    let name_sed = sed_replacement_escape(name);
    let cur_res = connector
        .execute_shell(&format!("sysctl -n {name_q}"))
        .await
        .map_err(TaskModuleError::from_connector)?;
    let cur_val = if cur_res.exit_code == 0 {
        String::from_utf8_lossy(&cur_res.stdout).trim().to_string()
    } else {
        String::new()
    };
    let file_pattern = shell_single_quote(&format!("^{name}[[:space:]]*="));
    let file_has_exact = {
        let check = connector
            .execute_shell(&format!("grep -E {file_pattern} {file_q} >/dev/null 2>&1"))
            .await
            .map_err(TaskModuleError::from_connector)?;
        if check.exit_code == 0 {
            let read_cmd = format!(
                "grep -E {file_pattern} {file_q} 2>/dev/null | tail -n1 | sed -E 's/^[^=]*=[[:space:]]*//'"
            );
            let read_res = connector
                .execute_shell(&read_cmd)
                .await
                .map_err(TaskModuleError::from_connector)?;
            String::from_utf8_lossy(&read_res.stdout).trim() == value
        } else {
            false
        }
    };
    let mut changed = false;
    let runtime_ok = cur_val == value;
    if !runtime_ok || !file_has_exact {
        let write_cmd = format!(
            "grep -q {name_line} {file_q} && sed -i 's|^{name_sed} *=.*|{name_sed} = {value_sed}|' {file_q} || echo {line} >> {file_q}",
            name_line = shell_single_quote(&format!("^{name} *=")),
            line = shell_single_quote(&format!("{name} = {value}")),
        );
        let res = connector
            .execute_shell(&write_cmd)
            .await
            .map_err(TaskModuleError::from_connector)?;
        if res.exit_code != 0 {
            let stderr_str = String::from_utf8_lossy(&res.stderr);
            return Err(TaskModuleError::Message(format!(
                "failed to write kernel param to {sysctl_file}: {stderr_str}"
            )));
        }
        changed = true;
    }
    if !runtime_ok {
        let apply_cmd = format!(
            "sysctl -w {assign} || sysctl -p {file_q}",
            assign = shell_single_quote(&format!("{name}={value}")),
        );
        let res = connector
            .execute_shell(&apply_cmd)
            .await
            .map_err(TaskModuleError::from_connector)?;
        if res.exit_code != 0 {
            let stderr_str = String::from_utf8_lossy(&res.stderr);
            return Err(TaskModuleError::Message(format!(
                "sysctl reload failed: {stderr_str}"
            )));
        }
        changed = true;
    }
    Ok(Some(TaskResult {
        stdout: format!("sysctl set {name} = {value}"),
        stderr: String::new(),
        exit_code: 0,
        changed,
        failed: false,
    }))
}
