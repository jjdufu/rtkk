use super::remote_fs::{atomic_put_local_path, chmod_remote, octal_modes_equal};
use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::connector::{Connector, unique_tmp_id};
use crate::utils::{parse_file_mode, shell_single_quote};
use serde_json::Value;
use sha2::Digest as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn http_timeout_secs() -> u64 {
    match std::env::var("RTKK_HTTP_TIMEOUT_SECS") {
        Ok(v) => v.trim().parse::<u64>().unwrap_or(600).max(1),
        Err(_) => 600,
    }
}

async fn verify_existing_local_file(dest_path: PathBuf, sha256_opt: Option<String>) -> bool {
    tokio::task::spawn_blocking(move || {
        let dest = dest_path.as_path();
        if !dest.exists() {
            return false;
        }
        if let Some(ref expected_sha) = sha256_opt {
            let Ok(mut file) = std::fs::File::open(dest) else {
                return false;
            };
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
            let hash_hex = format!("{:x}", hasher.finalize());
            hash_hex.eq_ignore_ascii_case(expected_sha.trim())
        } else {
            std::fs::metadata(dest).is_ok_and(|m| m.len() > 0)
        }
    })
    .await
    .unwrap_or(false)
}

fn report_download_progress(
    host: &str,
    task_name: &str,
    downloaded: u64,
    total_bytes: Option<u64>,
    elapsed_secs: f64,
) {
    let speed_str = if elapsed_secs > 0.0 {
        let speed_kbps = (crate::utils::u64_to_f64(downloaded) / elapsed_secs) / 1024.0;
        if speed_kbps >= 1024.0 {
            format!("{:.1} MB/s", speed_kbps / 1024.0)
        } else {
            format!("{speed_kbps:.0} KB/s")
        }
    } else {
        "0 KB/s".to_string()
    };
    let dl_tenth_mb = u32::try_from(downloaded.saturating_mul(10) / (1024 * 1024)).unwrap_or(0);
    let dl_mb = f64::from(dl_tenth_mb) / 10.0;
    if let Some(total) = total_bytes {
        let pct = downloaded
            .saturating_mul(100)
            .checked_div(total)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        let tot_tenth_mb = u32::try_from(total.saturating_mul(10) / (1024 * 1024)).unwrap_or(0);
        let tot_mb = f64::from(tot_tenth_mb) / 10.0;
        crate::logging::update_task(
            host,
            task_name,
            pct,
            &format!("{dl_mb:.1}MB/{tot_mb:.1}MB, {speed_str}"),
        );
    } else {
        crate::logging::update_task(host, task_name, 0, &format!("{dl_mb:.1}MB, {speed_str}"));
    }
}

async fn download_once(
    client: &reqwest::Client,
    url: &str,
    temp_dest: &Path,
    sha256_opt: Option<&str>,
    host: &str,
    task_name: &str,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt as _;

    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("http download failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "http download failed: status code {status} for URL {url}"
        ));
    }
    let mut file = tokio::fs::File::create(temp_dest)
        .await
        .map_err(|e| format!("failed to create temp file {}: {e}", temp_dest.display()))?;
    let mut hasher = sha256_opt.map(|_| sha2::Sha256::new());
    let total_bytes = response.content_length();
    let mut downloaded = 0u64;
    let start_time = std::time::Instant::now();

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("failed to read downloaded chunk: {e}"))?
    {
        if let Some(ref mut h) = hasher {
            h.update(&chunk);
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("failed to write to temp file: {e}"))?;
        downloaded = downloaded.saturating_add(u64::try_from(chunk.len()).unwrap_or(0));
        report_download_progress(
            host,
            task_name,
            downloaded,
            total_bytes,
            start_time.elapsed().as_secs_f64(),
        );
    }
    file.sync_all()
        .await
        .map_err(|e| format!("failed to sync file: {e}"))?;
    drop(file);

    if let (Some(expected_sha), Some(h)) = (sha256_opt, hasher) {
        let hash_hex = format!("{:x}", h.finalize());
        if !hash_hex.eq_ignore_ascii_case(expected_sha.trim()) {
            return Err(format!(
                "SHA256 checksum verification failed for {url}. Expected: {expected_sha}, Got: {hash_hex}"
            ));
        }
    }
    Ok(())
}

async fn run_download_loop(
    task: &Task,
    url: &str,
    temp_dest: &Path,
    sha256_opt: Option<&str>,
    context: &gtmpl::Value,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(http_timeout_secs()))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))?;
    let json_context = crate::template::gtmpl_to_json(context);
    let host = json_context
        .get("inventory_hostname")
        .and_then(Value::as_str)
        .unwrap_or("localhost");
    let task_name = task.name.as_deref().unwrap_or("http_get_file");

    let attempts = 3;
    let mut last_err = String::new();
    for attempt in 1..=attempts {
        match download_once(&client, url, temp_dest, sha256_opt, host, task_name).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                let _ = tokio::fs::remove_file(temp_dest).await;
                last_err = e;
                if attempt < attempts {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }
    Err(format!(
        "failed to download {url} after {attempts} attempts: {last_err}"
    ))
}

async fn check_existing_on_target(
    connector: &Connector,
    dest: &str,
    sha256_opt: Option<&str>,
    mode: u32,
) -> Result<Option<bool>, TaskModuleError> {
    let dest_q = shell_single_quote(dest);
    if let Connector::Local(_) = connector {
        let valid =
            verify_existing_local_file(PathBuf::from(dest), sha256_opt.map(ToString::to_string))
                .await;
        if !valid {
            return Ok(None);
        }
        let stat_res = connector
            .execute_shell(&format!("stat -c '%a' {dest_q} 2>/dev/null"))
            .await
            .map_err(TaskModuleError::from_connector)?;
        let cur_mode = String::from_utf8_lossy(&stat_res.stdout);
        if stat_res.exit_code == 0 && !octal_modes_equal(cur_mode.trim(), mode) {
            chmod_remote(connector, dest, mode)
                .await
                .map_err(TaskModuleError::from_connector)?;
            return Ok(Some(true));
        }
        return Ok(Some(false));
    }

    let check_cmd = if sha256_opt.is_some() {
        format!(
            "[ -f {dest_q} ] && stat -c '%a' {dest_q} 2>/dev/null && sha256sum {dest_q} 2>/dev/null"
        )
    } else {
        format!("[ -s {dest_q} ] && stat -c '%a' {dest_q} 2>/dev/null")
    };
    let res = connector
        .execute_shell(&check_cmd)
        .await
        .map_err(TaskModuleError::from_connector)?;
    if res.exit_code != 0 {
        return Ok(None);
    }
    let out = String::from_utf8_lossy(&res.stdout);
    let mut lines = out.lines();
    let Some(remote_mode) = lines.next() else {
        return Ok(None);
    };
    if let Some(expected_sha) = sha256_opt {
        let remote_sha = lines
            .next()
            .and_then(|l| l.split_whitespace().next())
            .unwrap_or("");
        if !remote_sha.eq_ignore_ascii_case(expected_sha.trim()) {
            return Ok(None);
        }
    }
    if octal_modes_equal(remote_mode, mode) {
        Ok(Some(false))
    } else {
        chmod_remote(connector, dest, mode)
            .await
            .map_err(TaskModuleError::from_connector)?;
        Ok(Some(true))
    }
}

async fn download_to_local_target(
    task: &Task,
    connector: &Connector,
    url: &str,
    dest: &str,
    sha256_opt: Option<&str>,
    mode: u32,
    context: &gtmpl::Value,
) -> Result<(), TaskModuleError> {
    let (pid, rand_id) = unique_tmp_id();
    let escalated = connector.privilege().is_some_and(|p| p.become_enabled);
    if escalated {
        let tmp_local = std::env::temp_dir().join(format!(".rtkk_http.{pid}.{rand_id}.rtkk.part"));
        run_download_loop(task, url, &tmp_local, sha256_opt, context)
            .await
            .map_err(TaskModuleError::Message)?;
        let put_res = atomic_put_local_path(connector, &tmp_local, dest, mode).await;
        let _ = tokio::fs::remove_file(&tmp_local).await;
        return put_res.map_err(TaskModuleError::from_connector);
    }
    let dest_path = PathBuf::from(dest);
    if let Some(parent) = dest_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| TaskModuleError::Message(format!("failed to create local dir: {e}")))?;
    }
    let part_path = PathBuf::from(format!("{dest}.rtkk.part.{pid}.{rand_id}"));
    run_download_loop(task, url, &part_path, sha256_opt, context)
        .await
        .map_err(TaskModuleError::Message)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let perms = std::fs::Permissions::from_mode(mode);
        if let Err(e) = tokio::fs::set_permissions(&part_path, perms).await {
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(TaskModuleError::Message(format!(
                "failed to set permissions on {}: {e}",
                part_path.display()
            )));
        }
    }
    if let Err(e) = tokio::fs::rename(&part_path, &dest_path).await {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(TaskModuleError::Message(format!(
            "atomic rename failed: {e}"
        )));
    }
    Ok(())
}

pub(crate) async fn execute_http_get_file(
    task: &Task,
    connector: &Connector,
    context: &gtmpl::Value,
) -> Result<Option<TaskResult>, TaskModuleError> {
    let Some(ref http_config) = task.http_get_file else {
        return Ok(None);
    };
    let rendered = render_value(http_config, context)?;
    let url = rendered
        .get("url")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| TaskModuleError::Message("http_get_file: missing url".to_string()))?;
    let dest = rendered
        .get("dest")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| TaskModuleError::Message("http_get_file: missing dest".to_string()))?;
    let sha256_opt = rendered
        .get("sha256")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mode = parse_file_mode(rendered.get("mode"), 0o644)?;

    if let Some(mode_changed) = check_existing_on_target(connector, dest, sha256_opt, mode).await? {
        return Ok(Some(TaskResult {
            stdout: format!("file already exists and is valid: {dest}"),
            stderr: String::new(),
            exit_code: 0,
            changed: mode_changed,
            failed: false,
        }));
    }

    match connector {
        Connector::Local(_) => {
            download_to_local_target(task, connector, url, dest, sha256_opt, mode, context).await?;
        }
        Connector::Ssh(_) => {
            let (pid, rand_id) = unique_tmp_id();
            let tmp_local =
                std::env::temp_dir().join(format!(".rtkk_http.{pid}.{rand_id}.rtkk.part"));
            run_download_loop(task, url, &tmp_local, sha256_opt, context)
                .await
                .map_err(TaskModuleError::Message)?;
            let put_res = atomic_put_local_path(connector, &tmp_local, dest, mode).await;
            let _ = tokio::fs::remove_file(&tmp_local).await;
            put_res.map_err(TaskModuleError::from_connector)?;
        }
    }

    Ok(Some(TaskResult {
        stdout: format!("downloaded {url} -> {dest}"),
        stderr: String::new(),
        exit_code: 0,
        changed: true,
        failed: false,
    }))
}
