use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::Connector;
use crate::utils::shell_single_quote;
use serde_json::Value;
use sha2::Digest as _;
use std::path::Path;
use tokio::io::AsyncReadExt as _;
async fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = tokio::fs::File::open(path)
        .await
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .await
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        if let Some(slice) = buf.get(..n) {
            hasher.update(slice);
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}
pub(crate) async fn execute_fetch(
    task: &Task,
    connector: &Connector,
    context: &gtmpl::Value,
) -> Result<Option<TaskResult>, TaskModuleError> {
    let Some(ref fetch_config) = task.fetch else {
        return Ok(None);
    };
    let rendered = render_value(fetch_config, context)?;
    let src = rendered
        .get("src")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("fetch: missing src".to_string()))?;
    let dest = rendered
        .get("dest")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("fetch: missing dest".to_string()))?;
    let dest_path = Path::new(dest);
    let mut is_same = false;
    if dest_path.exists() {
        if let Ok(local_sha) = sha256_file(dest_path).await {
            let src_q = shell_single_quote(src);
            let remote_sha_cmd = format!("sha256sum {src_q} 2>/dev/null");
            match connector.execute_shell(&remote_sha_cmd).await {
                Ok(remote_res) if remote_res.exit_code == 0 => {
                    let out_str = String::from_utf8_lossy(&remote_res.stdout);
                    if let Some(remote_sha) = out_str.split_whitespace().next() {
                        if remote_sha.trim() == local_sha.trim() {
                            is_same = true;
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => return Err(TaskModuleError::from_connector(e)),
            }
        }
    }
    if is_same {
        return Ok(Some(TaskResult {
            stdout: format!("file already fetched and is identical: {dest}"),
            stderr: String::new(),
            exit_code: 0,
            changed: false,
            failed: false,
        }));
    }
    if let Some(parent) = dest_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| TaskModuleError::Message(format!("failed to create local dir: {e}")))?;
    }
    connector
        .fetch_file(Path::new(src), dest_path)
        .await
        .map_err(TaskModuleError::from_connector)?;
    Ok(Some(TaskResult {
        stdout: format!("fetched {src} -> {dest}"),
        stderr: String::new(),
        exit_code: 0,
        changed: true,
        failed: false,
    }))
}
