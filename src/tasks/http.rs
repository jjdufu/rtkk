use super::{Task, TaskModuleError, TaskResult, render_value};
use serde_json::Value;
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::time::Duration;
async fn verify_existing_file_async(dest_path: PathBuf, sha256_opt: Option<String>) -> bool {
    tokio::task::spawn_blocking(move || {
        let dest = dest_path.as_path();
        if !dest.exists() {
            return false;
        }
        if let Some(ref expected_sha) = sha256_opt {
            if let Ok(mut file) = std::fs::File::open(dest) {
                use std::io::Read as _;
                let mut hasher = sha2::Sha256::new();
                let mut buffer = vec![0u8; 65536];
                loop {
                    match file.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => {
                            if let Some(slice) = buffer.get(..n) {
                                hasher.update(slice);
                            } else {
                                return false;
                            }
                        }
                        Err(_) => return false,
                    }
                }
                let result_hash = hasher.finalize();
                let hash_hex = format!("{result_hash:x}");
                hash_hex == expected_sha.trim().to_lowercase()
            } else {
                false
            }
        } else {
            std::fs::metadata(dest).is_ok_and(|m| m.len() > 0)
        }
    })
    .await
    .unwrap_or(false)
}
fn http_timeout_secs() -> u64 {
    match std::env::var("RTKK_HTTP_TIMEOUT_SECS") {
        Ok(v) => v.trim().parse::<u64>().unwrap_or(600).max(1),
        Err(_) => 600,
    }
}
async fn run_download_loop(
    task: &Task,
    url: &str,
    temp_dest: &Path,
    sha256_opt: Option<&str>,
    context: &gtmpl::Value,
) -> Result<(bool, String), String> {
    use tokio::io::AsyncWriteExt as _;
    let attempts = 3;
    let mut success = false;
    let mut last_err = String::new();
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(http_timeout_secs()))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))?;
    for attempt in 1..=attempts {
        let res = async {
            let mut response = client
                .get(url)
                .send()
                .await
                .map_err(|e| format!("http download failed: {e}"))?;
            let status = response.status();
            if !status.is_success() {
                return Err(format!("http download failed: status code {status} for URL {url}"));
            }
            let mut file = tokio::fs::File::create(temp_dest).await
                .map_err(|e| format!("failed to create temp file: {e}"))?;
            let mut hasher = sha256_opt.as_ref().map(|_| sha2::Sha256::new());
            let total_bytes = response.content_length();
            let mut downloaded = 0u64;
            let json_context = crate::template::gtmpl_to_json(context);
            let host = json_context.get("inventory_hostname").and_then(|v| v.as_str()).unwrap_or("localhost");
            let task_name = task.name.as_deref().unwrap_or("http_get_file");
            let start_time = std::time::Instant::now();
            while let Some(chunk) = response.chunk().await.map_err(|e| format!("failed to read downloaded chunk: {e}"))? {
                if let Some(ref mut h) = hasher {
                    h.update(&chunk);
                }
                file.write_all(&chunk).await
                    .map_err(|e| format!("failed to write to temp file: {e}"))?;
                downloaded += u64::try_from(chunk.len()).unwrap_or(0);
                let elapsed = start_time.elapsed().as_secs_f64();
                let speed_str = if elapsed > 0.0 {
                    let downloaded_f64 = crate::utils::u64_to_f64(downloaded);
                    let speed_kbps = (downloaded_f64 / elapsed) / 1024.0;
                    if speed_kbps >= 1024.0 {
                        format!("{speed_mbps:.1} MB/s", speed_mbps = speed_kbps / 1024.0)
                    } else {
                        format!("{speed_kbps:.0} KB/s")
                    }
                } else {
                    "0 KB/s".to_string()
                };
                if let Some(total) = total_bytes {
                    let pct = (downloaded * 100).checked_div(total)
                        .and_then(|v| u32::try_from(v).ok())
                        .unwrap_or(0);
                    let downloaded_tenth_mb = u32::try_from(downloaded * 10 / (1024 * 1024)).unwrap_or(0);
                    let total_tenth_mb = u32::try_from(total * 10 / (1024 * 1024)).unwrap_or(0);
                    let downloaded_mb = f64::from(downloaded_tenth_mb) / 10.0;
                    let total_mb = f64::from(total_tenth_mb) / 10.0;
                    let text = format!("{downloaded_mb:.1}MB/{total_mb:.1}MB, {speed_str}");
                    crate::logging::update_task(host, task_name, pct, &text);
                } else {
                    let downloaded_tenth_mb = u32::try_from(downloaded * 10 / (1024 * 1024)).unwrap_or(0);
                    let downloaded_mb = f64::from(downloaded_tenth_mb) / 10.0;
                    let text = format!("{downloaded_mb:.1}MB, {speed_str}");
                    crate::logging::update_task(host, task_name, 0, &text);
                }
            }
            file.sync_all().await.map_err(|e| format!("failed to sync file: {e}"))?;
            drop(file);
            if let Some(expected_sha) = sha256_opt {
                if let Some(h) = hasher {
                    let result_hash = h.finalize();
                    let hash_hex = format!("{result_hash:x}");
                    if hash_hex != expected_sha.trim().to_lowercase() {
                        let _ = tokio::fs::remove_file(temp_dest).await;
                        return Err(format!("SHA256 checksum verification failed for {url}. Expected: {expected_sha}, Got: {hash_hex}"));
                    }
                }
            }
            Ok(())
        }.await;
        match res {
            Ok(()) => {
                success = true;
                break;
            }
            Err(e) => {
                last_err = e;
                if attempt < attempts {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    }
    Ok((success, last_err))
}
pub(crate) async fn execute_http_get_file(
    task: &Task,
    context: &gtmpl::Value,
) -> Result<Option<TaskResult>, TaskModuleError> {
    let Some(ref http_config) = task.http_get_file else {
        return Ok(None);
    };
    let rendered = render_value(http_config, context)?;
    let url = rendered
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("http_get_file: missing url".to_string()))?;
    let dest = rendered
        .get("dest")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("http_get_file: missing dest".to_string()))?;
    let sha256_opt = rendered.get("sha256").and_then(Value::as_str);
    let dest_path = PathBuf::from(dest);
    if verify_existing_file_async(dest_path.clone(), sha256_opt.map(String::from)).await {
        return Ok(Some(TaskResult {
            stdout: format!("file already exists and is valid: {dest}"),
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
    let temp_dest = dest_path.with_extension("tmp");
    let (success, last_err) = run_download_loop(task, url, &temp_dest, sha256_opt, context)
        .await
        .map_err(TaskModuleError::Message)?;
    if !success {
        return Err(TaskModuleError::Message(format!(
            "failed to download {url} after 3 attempts: {last_err}"
        )));
    }
    tokio::fs::rename(&temp_dest, &dest_path)
        .await
        .map_err(|e| TaskModuleError::Message(format!("atomic rename failed: {e}")))?;
    Ok(Some(TaskResult {
        stdout: format!("downloaded {url} -> {dest}"),
        stderr: String::new(),
        exit_code: 0,
        changed: true,
        failed: false,
    }))
}
