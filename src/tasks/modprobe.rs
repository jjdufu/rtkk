use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::Connector;
use crate::utils::{is_safe_module_name, shell_single_quote};
use serde_json::Value;

async fn is_module_loaded(connector: &Connector, norm: &str) -> Result<bool, TaskModuleError> {
    let sys_dir = format!("/sys/module/{norm}");
    let sys_q = shell_single_quote(&sys_dir);
    let sys_res = connector
        .execute_shell(&format!("[ -d {sys_q} ]"))
        .await
        .map_err(TaskModuleError::from_connector)?;
    if sys_res.exit_code == 0 {
        return Ok(true);
    }
    let proc_res = connector
        .execute_shell("cat /proc/modules 2>/dev/null || lsmod 2>/dev/null")
        .await
        .map_err(TaskModuleError::from_connector)?;
    if proc_res.exit_code != 0 {
        return Ok(false);
    }
    let modules_list = String::from_utf8_lossy(&proc_res.stdout);
    Ok(modules_list.lines().any(|line| {
        line.split_whitespace()
            .next()
            .is_some_and(|m| m != "Module" && m.replace('-', "_") == norm)
    }))
}

async fn ensure_present(
    connector: &Connector,
    name: &str,
    name_q: &str,
    conf_q: &str,
    loaded: bool,
) -> Result<bool, TaskModuleError> {
    let mut changed = false;
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
    let check_conf = connector
        .execute_shell(&format!("grep -qxF -- {name_q} {conf_q} 2>/dev/null"))
        .await
        .map_err(TaskModuleError::from_connector)?;
    if check_conf.exit_code != 0 {
        let write_cmd =
            format!("mkdir -p /etc/modules-load.d && printf '%s\\n' {name_q} > {conf_q}");
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
    Ok(changed)
}

async fn ensure_absent(
    connector: &Connector,
    name: &str,
    name_q: &str,
    conf_q: &str,
    loaded: bool,
) -> Result<bool, TaskModuleError> {
    let mut changed = false;
    if loaded {
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
        changed = true;
    }
    let rm_cmd = format!("if [ -e {conf_q} ]; then rm -f {conf_q} && printf 'REMOVED'; fi");
    let rm_res = connector
        .execute_shell(&rm_cmd)
        .await
        .map_err(TaskModuleError::from_connector)?;
    if rm_res.exit_code != 0 {
        let stderr_str = String::from_utf8_lossy(&rm_res.stderr);
        return Err(TaskModuleError::Message(format!(
            "failed to remove modules-load.d config for {name}: {stderr_str}"
        )));
    }
    if String::from_utf8_lossy(&rm_res.stdout).contains("REMOVED") {
        changed = true;
    }
    Ok(changed)
}

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
    if state != "present" && state != "absent" {
        return Err(TaskModuleError::Message(format!(
            "modprobe: unsupported state '{state}' (expected 'present' or 'absent')"
        )));
    }
    let norm = name.replace('-', "_");
    let loaded = is_module_loaded(connector, &norm).await?;
    let name_q = shell_single_quote(name);
    let conf_path = format!("/etc/modules-load.d/{name}.conf");
    let conf_q = shell_single_quote(&conf_path);

    let changed = if state == "present" {
        ensure_present(connector, name, &name_q, &conf_q, loaded).await?
    } else {
        ensure_absent(connector, name, &name_q, &conf_q, loaded).await?
    };

    Ok(Some(TaskResult {
        stdout: format!("modprobe module '{name}' state '{state}' ensured"),
        stderr: String::new(),
        exit_code: 0,
        changed,
        failed: false,
    }))
}
