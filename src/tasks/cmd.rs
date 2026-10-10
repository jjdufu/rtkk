use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::Connector;
use serde_json::Value;
pub(crate) async fn execute_cmd_shell(
    task: &Task,
    connector: &Connector,
    context: &gtmpl::Value,
) -> Result<Option<TaskResult>, TaskModuleError> {
    let is_shell = task.shell.is_some();
    let is_cmd = task.command.is_some();
    if !is_cmd && !is_shell {
        return Ok(None);
    }
    let config_val = if is_shell {
        task.shell.as_ref().ok_or("shell is missing")?
    } else {
        task.command.as_ref().ok_or("command is missing")?
    };
    let rendered = render_value(config_val, context)?;
    let cmd_str = if let Some(s) = rendered.as_str() {
        s.to_string()
    } else if let Some(m) = rendered.as_object() {
        m.get("_raw_params")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    } else {
        return Err(TaskModuleError::Message(
            "command/shell: invalid arguments".to_string(),
        ));
    };
    let res = if is_shell {
        connector
            .execute_shell(&cmd_str)
            .await
            .map_err(TaskModuleError::from_connector)?
    } else {
        let argv = shell_words::split(&cmd_str).map_err(|e| {
            TaskModuleError::Message(format!("failed to parse command argv: {e}"))
        })?;
        if argv.is_empty() {
            return Err(TaskModuleError::Message("empty command".to_string()));
        }
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        connector
            .execute_argv(&refs)
            .await
            .map_err(TaskModuleError::from_connector)?
    };
    let out = String::from_utf8_lossy(&res.stdout).to_string();
    let err = String::from_utf8_lossy(&res.stderr).to_string();
    let failed = res.exit_code != 0;
    let mut changed = !failed;
    if let Some(ref cw) = task.changed_when {
        let mut custom_context = crate::template::gtmpl_to_json(context);
        if let Some(obj) = custom_context.as_object_mut() {
            obj.insert("stdout".to_string(), Value::String(out.clone()));
            obj.insert("stderr".to_string(), Value::String(err.clone()));
            obj.insert("rc".to_string(), Value::Number(res.exit_code.into()));
        }
        changed = crate::playbook::eval_condition(cw, &custom_context);
    }
    Ok(Some(TaskResult {
        stdout: out,
        stderr: err,
        exit_code: res.exit_code,
        changed,
        failed,
    }))
}
