use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::Connector;
use crate::utils::shell_single_quote;
use serde_json::Value;

fn validate_unit_name(name: &str) -> Result<(), TaskModuleError> {
    if name.is_empty() {
        return Err(TaskModuleError::Message(
            "systemd: unit name cannot be empty".to_string(),
        ));
    }
    if name.starts_with('-') {
        return Err(TaskModuleError::Message(format!(
            "systemd: invalid unit name '{name}': cannot start with '-'"
        )));
    }
    let valid = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@' | ':' | '\\'));
    if !valid {
        return Err(TaskModuleError::Message(format!(
            "systemd: invalid unit name '{name}'"
        )));
    }
    Ok(())
}

fn validate_state(state: &str) -> Result<(), TaskModuleError> {
    match state {
        "started" | "stopped" | "restarted" | "reloaded" => Ok(()),
        _ => Err(TaskModuleError::Message(format!(
            "unsupported systemd state '{state}' (expected 'started', 'stopped', 'restarted', or 'reloaded')"
        ))),
    }
}

pub(crate) async fn execute_systemd(
    task: &Task,
    connector: &Connector,
    context: &gtmpl::Value,
) -> Result<Option<TaskResult>, TaskModuleError> {
    let Some(ref systemd_config) = task.systemd else {
        return Ok(None);
    };
    let rendered = render_value(systemd_config, context)?;

    let state = match rendered.get("state") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            validate_state(s)?;
            Some(s.as_str())
        }
        Some(_) => {
            return Err(TaskModuleError::Message(
                "systemd: 'state' must be a string".to_string(),
            ));
        }
    };
    let enabled = match rendered.get("enabled") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => {
            return Err(TaskModuleError::Message(
                "systemd: 'enabled' must be a boolean".to_string(),
            ));
        }
    };
    let daemon_reload = match rendered.get("daemon_reload") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err(TaskModuleError::Message(
                "systemd: 'daemon_reload' must be a boolean".to_string(),
            ));
        }
    };

    let name_opt = match rendered.get("name") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            validate_unit_name(s)?;
            Some(s.as_str())
        }
        Some(_) => {
            return Err(TaskModuleError::Message(
                "systemd: 'name' must be a string".to_string(),
            ));
        }
    };
    if (state.is_some() || enabled.is_some() || !daemon_reload) && name_opt.is_none() {
        return Err(TaskModuleError::Message(
            "systemd: missing name".to_string(),
        ));
    }

    let mut changed = false;
    if daemon_reload {
        run_systemctl(connector, "systemctl daemon-reload", "daemon-reload").await?;
        changed = true;
    }
    if let Some(name) = name_opt {
        let name_q = shell_single_quote(name);
        if let Some(enable) = enabled {
            changed |= apply_enabled(connector, &name_q, enable).await?;
        }
        if let Some(st) = state {
            changed |= apply_state(connector, &name_q, st).await?;
        }
    }

    let label = name_opt.unwrap_or("daemon-reload");
    Ok(Some(TaskResult {
        stdout: format!("systemd '{label}' processed"),
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
        "reloaded" => Some("reload"),
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
