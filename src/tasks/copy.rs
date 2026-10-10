use super::remote_fs::{atomic_put_bytes, atomic_put_local_path};
use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::Connector;
use crate::utils::shell_single_quote;
use serde_json::Value;
use sha2::Digest as _;
use std::path::Path;
async fn is_copy_identical(
    connector: &Connector,
    dest: &str,
    file_data: &[u8],
    mode: u32,
) -> Result<bool, TaskModuleError> {
    let mut hasher = sha2::Sha256::new();
    hasher.update(file_data);
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
pub(crate) async fn execute_copy(
    task: &Task,
    connector: &Connector,
    context: &gtmpl::Value,
) -> Result<Option<TaskResult>, TaskModuleError> {
    let Some(ref copy_config) = task.copy else {
        return Ok(None);
    };
    let rendered = render_value(copy_config, context)?;
    let dest = rendered
        .get("dest")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("copy: missing dest".to_string()))?;
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
    let content_opt = rendered.get("content").and_then(Value::as_str);
    let src_opt = rendered.get("src").and_then(Value::as_str);
    let file_data = if let Some(content) = content_opt {
        content.as_bytes().to_vec()
    } else if let Some(src) = src_opt {
        tokio::fs::read(src)
            .await
            .map_err(|e| TaskModuleError::Message(format!("failed to read local src {src}: {e}")))?
    } else {
        return Err(TaskModuleError::Message(
            "copy: must specify either content or src".to_string(),
        ));
    };
    if is_copy_identical(connector, dest, &file_data, mode).await? {
        return Ok(Some(TaskResult {
            stdout: format!("file already exists and is identical: {dest}"),
            stderr: String::new(),
            exit_code: 0,
            changed: false,
            failed: false,
        }));
    }
    if content_opt.is_some() {
        atomic_put_bytes(connector, &file_data, dest, mode)
            .await
            .map_err(TaskModuleError::from_connector)?;
    } else if let Some(src) = src_opt {
        atomic_put_local_path(connector, Path::new(src), dest, mode)
            .await
            .map_err(TaskModuleError::from_connector)?;
    }
    Ok(Some(TaskResult {
        stdout: format!("copied successfully to {dest}"),
        stderr: String::new(),
        exit_code: 0,
        changed: true,
        failed: false,
    }))
}
