use super::remote_fs::mkdir_p;
use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::Connector;
use crate::utils::{regex_escape_literal, sed_replacement_escape, shell_single_quote};
use serde_json::Value;
use std::path::Path;

fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "1".to_string() } else { "0".to_string() }),
        _ => None,
    }
}

fn normalize_sysctl_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn escape_unescaped_slash(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut trailing_backslashes: usize = 0;
    for ch in input.chars() {
        if ch == '/' && trailing_backslashes.is_multiple_of(2) {
            out.push('\\');
        }
        out.push(ch);
        if ch == '\\' {
            trailing_backslashes = trailing_backslashes.saturating_add(1);
        } else {
            trailing_backslashes = 0;
        }
    }
    out
}

fn validate_sysctl_inputs(
    name: &str,
    value: &str,
    sysctl_file: &str,
) -> Result<(), TaskModuleError> {
    if name.is_empty()
        || name.contains(['\n', '\r', '\0', '='])
        || name.chars().any(char::is_whitespace)
    {
        return Err(TaskModuleError::Message(format!(
            "sysctl: invalid parameter name '{name}'"
        )));
    }
    if value.contains(['\n', '\r', '\0']) {
        return Err(TaskModuleError::Message(
            "sysctl: value must not contain newlines or NUL bytes".to_string(),
        ));
    }
    if sysctl_file.trim().is_empty() || sysctl_file.contains(['\n', '\r', '\0']) {
        return Err(TaskModuleError::Message(
            "sysctl: invalid sysctl_file path".to_string(),
        ));
    }
    Ok(())
}

async fn check_sysctl_file_exact(
    connector: &Connector,
    file_pattern_q: &str,
    file_q: &str,
    want_norm: &str,
) -> Result<bool, TaskModuleError> {
    let check = connector
        .execute_shell(&format!(
            "grep -E {file_pattern_q} {file_q} >/dev/null 2>&1"
        ))
        .await
        .map_err(TaskModuleError::from_connector)?;
    if check.exit_code != 0 {
        return Ok(false);
    }
    let read_cmd = format!(
        "grep -E {file_pattern_q} {file_q} 2>/dev/null | tail -n 1 | sed -E 's/^[^=]*=[[:space:]]*//'"
    );
    let read_res = connector
        .execute_shell(&read_cmd)
        .await
        .map_err(TaskModuleError::from_connector)?;
    if read_res.exit_code != 0 {
        return Ok(false);
    }
    let file_val = String::from_utf8_lossy(&read_res.stdout);
    Ok(normalize_sysctl_ws(&file_val) == want_norm)
}

async fn persist_sysctl_entry(
    connector: &Connector,
    name: &str,
    value: &str,
    sysctl_file: &str,
    file_pattern_q: &str,
    file_q: &str,
) -> Result<(), TaskModuleError> {
    let key_re = escape_unescaped_slash(&regex_escape_literal(name));
    let replacement = escape_unescaped_slash(&sed_replacement_escape(&format!("{name} = {value}")));
    let sed_expr_q = shell_single_quote(&format!(
        "s/^[[:space:]]*{key_re}[[:space:]]*=.*/{replacement}/"
    ));
    let line_q = shell_single_quote(&format!("{name} = {value}"));
    let write_cmd = format!(
        "if grep -Eq {file_pattern_q} {file_q} 2>/dev/null; then sed -i -E {sed_expr_q} {file_q}; else printf '%s\\n' {line_q} >> {file_q}; fi"
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
    Ok(())
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
    validate_sysctl_inputs(name, &value, sysctl_file)?;

    if let Some(parent) = Path::new(sysctl_file).parent() {
        mkdir_p(connector, &parent.to_string_lossy())
            .await
            .map_err(TaskModuleError::from_connector)?;
    }

    let name_q = shell_single_quote(name);
    let file_q = shell_single_quote(sysctl_file);
    let want_norm = normalize_sysctl_ws(&value);

    let cur_res = connector
        .execute_shell(&format!("sysctl -n {name_q}"))
        .await
        .map_err(TaskModuleError::from_connector)?;
    let runtime_ok = cur_res.exit_code == 0
        && normalize_sysctl_ws(&String::from_utf8_lossy(&cur_res.stdout)) == want_norm;

    let key_re = regex_escape_literal(name);
    let file_pattern_q = shell_single_quote(&format!("^[[:space:]]*{key_re}[[:space:]]*="));
    let file_has_exact =
        check_sysctl_file_exact(connector, &file_pattern_q, &file_q, &want_norm).await?;

    let mut changed = false;
    if !runtime_ok || !file_has_exact {
        persist_sysctl_entry(
            connector,
            name,
            &value,
            sysctl_file,
            &file_pattern_q,
            &file_q,
        )
        .await?;
        changed = true;
    }
    if !runtime_ok {
        let assign_q = shell_single_quote(&format!("{name}={value}"));
        let apply_cmd = format!("sysctl -w {assign_q} || sysctl -p {file_q}");
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
