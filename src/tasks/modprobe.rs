use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::Connector;
use crate::utils::{is_safe_module_name, shell_single_quote};
use serde_json::Value;
pub(crate) async fn execute_modprobe(
    task: &Task,
    connector: &Connector,
    context: &gtmpl::Value,
) -> Result<Option<TaskResult>, TaskModuleError> {
    let Some(ref modprobe_config) = task.modprobe else {
        return Ok(None);
    };
    let rendered = render_value(modprobe_config, context)?;
    let name = rendered
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("modprobe: missing name".to_string()))?;
    if !is_safe_module_name(name) {
        return Err(TaskModuleError::Message(format!(
            "modprobe: invalid module name '{name}' (allowed: [A-Za-z0-9_.-])"
        )));
    }
    let state = rendered
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("present");
    let name_q = shell_single_quote(name);
    let check_res = connector
        .execute_shell("cat /proc/modules")
        .await
        .map_err(TaskModuleError::from_connector)?;
    let modules_list = String::from_utf8_lossy(&check_res.stdout).to_string();
    let loaded = modules_list.lines().any(|l| {
        l.split_whitespace()
            .next()
            .is_some_and(|module| module == name)
    });
    let mut changed = false;
    if state == "present" {
        if !loaded {
            let res = connector
                .execute_shell(&format!("modprobe {name_q}"))
                .await
                .map_err(TaskModuleError::from_connector)?;
            if res.exit_code != 0 {
                let stderr_str = String::from_utf8_lossy(&res.stderr);
                return Err(TaskModuleError::Message(format!(
                    "modprobe {name} failed: {stderr_str}"
                )));
            }
            changed = true;
        }
        let conf_path = format!("/etc/modules-load.d/{name}.conf");
        let conf_q = shell_single_quote(&conf_path);
        let check_conf = connector
            .execute_shell(&format!("grep -q {name_q} {conf_q} 2>/dev/null"))
            .await
            .map_err(TaskModuleError::from_connector)?;
        if check_conf.exit_code != 0 {
            let write_cmd = format!("mkdir -p /etc/modules-load.d && echo {name_q} > {conf_q}");
            let res = connector
                .execute_shell(&write_cmd)
                .await
                .map_err(TaskModuleError::from_connector)?;
            if res.exit_code != 0 {
                let stderr_str = String::from_utf8_lossy(&res.stderr);
                return Err(TaskModuleError::Message(format!(
                    "failed to write modules-load.d config: {stderr_str}"
                )));
            }
            changed = true;
        }
    } else if state == "absent" && loaded {
        let res = connector
            .execute_shell(&format!("modprobe -r {name_q}"))
            .await
            .map_err(TaskModuleError::from_connector)?;
        if res.exit_code != 0 {
            let stderr_str = String::from_utf8_lossy(&res.stderr);
            return Err(TaskModuleError::Message(format!(
                "modprobe -r {name} failed: {stderr_str}"
            )));
        }
        let conf_path = format!("/etc/modules-load.d/{name}.conf");
        let conf_q = shell_single_quote(&conf_path);
        let _ = connector.execute_shell(&format!("rm -f {conf_q}")).await;
        changed = true;
    }
    Ok(Some(TaskResult {
        stdout: format!("modprobe module '{name}' state '{state}' ensured"),
        stderr: String::new(),
        exit_code: 0,
        changed,
        failed: false,
    }))
}
