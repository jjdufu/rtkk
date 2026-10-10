use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::Connector;
use crate::utils::shell_single_quote;
use serde_json::Value;
pub(crate) async fn execute_systemd(
    task: &Task,
    connector: &Connector,
    context: &gtmpl::Value,
) -> Result<Option<TaskResult>, TaskModuleError> {
    let Some(ref systemd_config) = task.systemd else {
        return Ok(None);
    };
    let rendered = render_value(systemd_config, context)?;
    let name = rendered
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("systemd: missing name".to_string()))?;
    let name_q = shell_single_quote(name);
    let state = rendered.get("state").and_then(Value::as_str);
    let enabled = rendered.get("enabled").and_then(Value::as_bool);
    let daemon_reload = rendered
        .get("daemon_reload")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut changed = false;
    if daemon_reload {
        run_systemctl(connector, "systemctl daemon-reload", "daemon-reload").await?;
        changed = true;
    }
    if let Some(enable) = enabled {
        changed |= apply_enabled(connector, &name_q, enable).await?;
    }
    if let Some(st) = state {
        changed |= apply_state(connector, &name_q, st).await?;
    }
    Ok(Some(TaskResult {
        stdout: format!("systemd '{name}' processed"),
        stderr: String::new(),
        exit_code: 0,
        changed,
        failed: false,
    }))
}
async fn run_systemctl(
    connector: &Connector,
    cmd: &str,
    label: &str,
) -> Result<(), TaskModuleError> {
    let res = connector
        .execute_shell(cmd)
        .await
        .map_err(TaskModuleError::from_connector)?;
    if res.exit_code != 0 {
        let stderr_str = String::from_utf8_lossy(&res.stderr);
        return Err(TaskModuleError::Message(format!(
            "{label} failed: {stderr_str}"
        )));
    }
    Ok(())
}
async fn apply_enabled(
    connector: &Connector,
    name_q: &str,
    enable: bool,
) -> Result<bool, TaskModuleError> {
    let is_enabled_res = connector
        .execute_shell(&format!("systemctl is-enabled {name_q}"))
        .await
        .map_err(TaskModuleError::from_connector)?;
    let already_enabled = is_enabled_res.exit_code == 0;
    if enable == already_enabled {
        return Ok(false);
    }
    let cmd = if enable {
        format!("systemctl enable {name_q}")
    } else {
        format!("systemctl disable {name_q}")
    };
    run_systemctl(connector, &cmd, "systemctl enable/disable").await?;
    Ok(true)
}
async fn apply_state(
    connector: &Connector,
    name_q: &str,
    st: &str,
) -> Result<bool, TaskModuleError> {
    let is_active_res = connector
        .execute_shell(&format!("systemctl is-active {name_q}"))
        .await
        .map_err(TaskModuleError::from_connector)?;
    let active = is_active_res.exit_code == 0;
    let action = match st {
        "started" => {
            if active {
                None
            } else {
                Some("start")
            }
        }
        "stopped" => {
            if active {
                Some("stop")
            } else {
                None
            }
        }
        "restarted" => Some("restart"),
        _ => {
            return Err(TaskModuleError::Message(format!(
                "unsupported systemd state '{st}'"
            )));
        }
    };
    if let Some(act) = action {
        run_systemctl(
            connector,
            &format!("systemctl {act} {name_q}"),
            &format!("systemctl {act}"),
        )
        .await?;
        return Ok(true);
    }
    Ok(false)
}
