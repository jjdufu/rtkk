use super::remote_fs::{
    atomic_put_bytes, atomic_put_local_path, chmod_remote, mkdir_p, octal_modes_equal,
    shell_sha256_cmd, shell_stat_octal_cmd,
};
use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::Connector;
use crate::utils::{parse_file_mode, shell_single_quote};
use serde_json::Value;
use sha2::Digest as _;
use std::path::Path;

pub(crate) enum RemoteCheckOutcome {
    Identical,
    ModeMismatch,
    ContentDiffersOrMissing,
}

pub(crate) async fn check_remote_sha_and_mode(
    connector: &Connector,
    dest: &str,
    expected_sha: &str,
    expected_mode: u32,
) -> Result<RemoteCheckOutcome, TaskModuleError> {
    let dest_q = shell_single_quote(dest);
    let stat_cmd = format!(
        "{} && {}",
        shell_stat_octal_cmd(&dest_q),
        shell_sha256_cmd(&dest_q)
    );
    let check_res = connector
        .execute_shell(&stat_cmd)
        .await
        .map_err(TaskModuleError::from_connector)?;
    if check_res.exit_code != 0 {
        return Ok(RemoteCheckOutcome::ContentDiffersOrMissing);
    }
    let out_str = String::from_utf8_lossy(&check_res.stdout);
    let mut lines = out_str.lines();
    let Some(remote_mode_str) = lines.next() else {
        return Ok(RemoteCheckOutcome::ContentDiffersOrMissing);
    };
    let Some(remote_sha_line) = lines.next() else {
        return Ok(RemoteCheckOutcome::ContentDiffersOrMissing);
    };
    let Some(remote_sha) = remote_sha_line.split_whitespace().next() else {
        return Ok(RemoteCheckOutcome::ContentDiffersOrMissing);
    };
    if !remote_sha.trim().eq_ignore_ascii_case(expected_sha.trim()) {
        return Ok(RemoteCheckOutcome::ContentDiffersOrMissing);
    }
    if octal_modes_equal(remote_mode_str, expected_mode) {
        Ok(RemoteCheckOutcome::Identical)
    } else {
        Ok(RemoteCheckOutcome::ModeMismatch)
    }
}

async fn sha256_local_file_streaming(path: &Path) -> Result<String, TaskModuleError> {
    let path_buf = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read as _;
        let mut file = std::fs::File::open(&path_buf).map_err(|e| {
            TaskModuleError::Message(format!(
                "failed to open local src {}: {e}",
                path_buf.display()
            ))
        })?;
        let mut hasher = sha2::Sha256::new();
        let mut buf = vec![0u8; 65536];
        loop {
            let n = file.read(&mut buf).map_err(|e| {
                TaskModuleError::Message(format!(
                    "failed to read local src {}: {e}",
                    path_buf.display()
                ))
            })?;
            if n == 0 {
                break;
            }
            if let Some(slice) = buf.get(..n) {
                hasher.update(slice);
            }
        }
        Ok(format!("{:x}", hasher.finalize()))
    })
    .await
    .map_err(|e| TaskModuleError::Message(format!("local sha256 task join failed: {e}")))?
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
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| TaskModuleError::Message("copy: missing dest".to_string()))?;
    let mode = parse_file_mode(rendered.get("mode"), 0o644)?;
    let content_opt = rendered.get("content").and_then(Value::as_str);
    let src_resolved = rendered
        .get("src")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(crate::utils::expand_tilde_path);
    if content_opt.is_some() && src_resolved.is_some() {
        return Err(TaskModuleError::Message(
            "copy: cannot specify both content and src".to_string(),
        ));
    }
    let local_sha = if let Some(content) = content_opt {
        let mut hasher = sha2::Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    } else if let Some(ref src_path) = src_resolved {
        sha256_local_file_streaming(src_path).await?
    } else {
        return Err(TaskModuleError::Message(
            "copy: must specify either content or src".to_string(),
        ));
    };

    match check_remote_sha_and_mode(connector, dest, &local_sha, mode).await? {
        RemoteCheckOutcome::Identical => {
            return Ok(Some(TaskResult {
                stdout: format!("file already exists and is identical: {dest}"),
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
                stdout: format!("updated file permissions on {dest}"),
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
    if let Some(content) = content_opt {
        atomic_put_bytes(connector, content.as_bytes(), dest, mode)
            .await
            .map_err(TaskModuleError::from_connector)?;
    } else if let Some(ref src_path) = src_resolved {
        atomic_put_local_path(connector, src_path, dest, mode)
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
