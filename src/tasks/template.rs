use super::remote_fs::atomic_put_bytes;
use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::Connector;
use crate::template::render_template_string;
use crate::utils::shell_single_quote;
use serde_json::Value;
use sha2::Digest as _;
use std::path::Path;
async fn is_template_identical(
    connector: &Connector,
    dest: &str,
    rendered_content: &str,
    mode: u32,
) -> Result<bool, TaskModuleError> {
    let mut hasher = sha2::Sha256::new();
    hasher.update(rendered_content.as_bytes());
    let local_sha = format!("{:x}", hasher.finalize());
    let dest_q = shell_single_quote(dest);
    let stat_cmd = format!("stat -c '%a' {dest_q} 2>/dev/null && sha256sum {dest_q} 2>/dev/null");
    let check_res = connector
        .execute_shell(&stat_cmd)
        .await
        .map_err(TaskModuleError::from_connector)?;
    if check_res.exit_code == 0 {
        let out_str = String::from_utf8_lossy(&check_res.stdout);
        let mut lines = out_str.lines();
        if let Some(remote_mode_str) = lines.next() {
            if let Some(remote_sha_line) = lines.next() {
                if let Some(remote_sha) = remote_sha_line.split_whitespace().next() {
                    let local_mode_octal = format!("{mode:o}");
                    let remote_mode_normalized = remote_mode_str.trim().trim_start_matches('0');
                    let local_mode_normalized = local_mode_octal.trim_start_matches('0');
                    if (remote_mode_normalized == local_mode_normalized
                        || (remote_mode_normalized.is_empty() && local_mode_normalized.is_empty()))
                        && remote_sha.trim() == local_sha.trim()
                    {
                        return Ok(true);
                    }
                }
            }
        }
    }
    Ok(false)
}
pub(crate) async fn execute_template(
    task: &Task,
    connector: &Connector,
    context: &gtmpl::Value,
) -> Result<Option<TaskResult>, TaskModuleError> {
    let Some(ref template_config) = task.template else {
        return Ok(None);
    };
    let rendered = render_value(template_config, context)?;
    let src = rendered
        .get("src")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("template: missing src".to_string()))?;
    let dest = rendered
        .get("dest")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("template: missing dest".to_string()))?;
    let mode = rendered
        .get("mode")
        .and_then(|v| {
            if let Some(s) = v.as_str() {
                let s_trimmed = s
                    .trim()
                    .trim_start_matches("0o")
                    .trim_start_matches("0O")
                    .trim_start_matches('0');
                if s_trimmed.is_empty() {
                    Some(0)
                } else {
                    u32::from_str_radix(s_trimmed, 8).ok()
                }
            } else {
                v.as_u64().and_then(|n| u32::try_from(n).ok())
            }
        })
        .unwrap_or(0o644);
    let inventory_dir = if let gtmpl::Value::Map(map) = context {
        map.get("inventory_dir").and_then(|v| {
            if let gtmpl::Value::String(s) = v {
                Some(s.as_str())
            } else {
                None
            }
        })
    } else {
        None
    };
    let src_path = Path::new(src);
    let mut resolved_src = src_path.to_path_buf();
    if !resolved_src.exists() {
        if let Some(inv_dir) = inventory_dir {
            if let Some(filename) = src_path.file_name() {
                let candidate = Path::new(inv_dir).join("templates").join(filename);
                if candidate.exists() {
                    resolved_src = candidate;
                }
            }
        }
    }
    let template_content = tokio::fs::read_to_string(&resolved_src)
        .await
        .map_err(|e| {
            let resolved_src_disp = resolved_src.display();
            TaskModuleError::Message(format!(
                "failed to read local template {resolved_src_disp}: {e}"
            ))
        })?;
    let rendered_content = render_template_string(&template_content, context)
        .map_err(|e| TaskModuleError::Message(format!("failed to render template: {e}")))?;
    if is_template_identical(connector, dest, &rendered_content, mode).await? {
        return Ok(Some(TaskResult {
            stdout: format!("templated file already exists and is identical: {dest}"),
            stderr: String::new(),
            exit_code: 0,
            changed: false,
            failed: false,
        }));
    }
    atomic_put_bytes(connector, rendered_content.as_bytes(), dest, mode)
        .await
        .map_err(TaskModuleError::from_connector)?;
    Ok(Some(TaskResult {
        stdout: format!("templated {src} -> {dest}"),
        stderr: String::new(),
        exit_code: 0,
        changed: true,
        failed: false,
    }))
}
