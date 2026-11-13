use super::copy::{RemoteCheckOutcome, check_remote_sha_and_mode};
use super::remote_fs::{atomic_put_bytes, chmod_remote, mkdir_p};
use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::Connector;
use crate::template::render_template_string;
use crate::utils::parse_file_mode;
use serde_json::Value;
use sha2::Digest as _;
use std::path::{Path, PathBuf};

fn resolve_template_source(src: &str, context: &gtmpl::Value) -> PathBuf {
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
    if src_path.exists() {
        return src_path.to_path_buf();
    }
    if let (Some(inv_dir), Some(filename)) = (inventory_dir, src_path.file_name()) {
        let candidate = Path::new(inv_dir).join("templates").join(filename);
        if candidate.exists() {
            return candidate;
        }
    }
    src_path.to_path_buf()
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
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| TaskModuleError::Message("template: missing src".to_string()))?;
    let dest = rendered
        .get("dest")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| TaskModuleError::Message("template: missing dest".to_string()))?;
    let mode = parse_file_mode(rendered.get("mode"), 0o644)?;
    let resolved_src = resolve_template_source(src, context);

    let template_content = tokio::fs::read_to_string(&resolved_src)
        .await
        .map_err(|e| {
            TaskModuleError::Message(format!(
                "failed to read local template {}: {e}",
                resolved_src.display()
            ))
        })?;
    let rendered_content = render_template_string(&template_content, context)
        .map_err(|e| TaskModuleError::Message(format!("failed to render template: {e}")))?;

    let mut hasher = sha2::Sha256::new();
    hasher.update(rendered_content.as_bytes());
    let local_sha = format!("{:x}", hasher.finalize());

    match check_remote_sha_and_mode(connector, dest, &local_sha, mode).await? {
        RemoteCheckOutcome::Identical => {
            return Ok(Some(TaskResult {
                stdout: format!("templated file already exists and is identical: {dest}"),
                stderr: String::new(),
                exit_code: 0,
                changed: false,
                failed: false,
            }));
        }
        RemoteCheckOutcome::ModeMismatch => {
            chmod_remote(connector, dest, mode)
                .await
                .map_err(TaskModuleError::from_connector)?;
            return Ok(Some(TaskResult {
                stdout: format!("updated templated file permissions on {dest}"),
                stderr: String::new(),
                exit_code: 0,
                changed: true,
                failed: false,
            }));
        }
        RemoteCheckOutcome::ContentDiffersOrMissing => {}
    }

    if let Some(parent) = Path::new(dest).parent() {
        mkdir_p(connector, &parent.to_string_lossy())
            .await
            .map_err(TaskModuleError::from_connector)?;
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
