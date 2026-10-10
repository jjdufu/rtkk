use super::remote_fs::{atomic_put_bytes, mkdir_p};
use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::cert::{CertPolicy, GenCertArgs, generate_certificate};
use crate::connector::Connector;
use crate::utils::shell_single_quote;
use serde_json::Value;
use sha2::Digest as _;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex as TokioMutex;
struct GenCertParams {
    args: GenCertArgs,
    out_key_remote: PathBuf,
    out_cert_remote: PathBuf,
    out_key_local: PathBuf,
    out_cert_local: PathBuf,
}
fn ca_locks() -> &'static TokioMutex<HashMap<PathBuf, Arc<TokioMutex<()>>>> {
    static LOCKS: OnceLock<TokioMutex<HashMap<PathBuf, Arc<TokioMutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| TokioMutex::new(HashMap::new()))
}
async fn with_ca_lock<F, T>(path: &Path, f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let key = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf());
    let lock = {
        let mut map = ca_locks().lock().await;
        Arc::clone(
            map.entry(key)
                .or_insert_with(|| Arc::new(TokioMutex::new(()))),
        )
    };
    let _guard = lock.lock().await;
    f.await
}
fn file_sha_or_none(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&data);
    Some(format!("{:x}", hasher.finalize()))
}
fn parse_gen_cert_params(
    task: &Task,
    host_vars: &Value,
    context: &gtmpl::Value,
) -> Result<Option<GenCertParams>, TaskModuleError> {
    let Some(ref cert_config) = task.gen_cert else {
        return Ok(None);
    };
    let rendered = render_value(cert_config, context)?;
    let cn = rendered
        .get("cn")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("gen_cert: missing cn".to_string()))?
        .to_string();
    let out_key_remote = PathBuf::from(
        rendered
            .get("out_key")
            .and_then(Value::as_str)
            .ok_or_else(|| TaskModuleError::Message("gen_cert: missing out_key".to_string()))?,
    );
    let out_cert_remote = PathBuf::from(
        rendered
            .get("out_cert")
            .and_then(Value::as_str)
            .ok_or_else(|| TaskModuleError::Message("gen_cert: missing out_cert".to_string()))?,
    );
    let hostname = host_vars
        .get("inventory_hostname")
        .and_then(Value::as_str)
        .unwrap_or("local");
    let is_ca = rendered
        .get("is_ca")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let key_filename = out_key_remote
        .file_name()
        .ok_or_else(|| TaskModuleError::Message("invalid out_key".to_string()))?
        .to_str()
        .ok_or_else(|| TaskModuleError::Message("invalid out_key string".to_string()))?;
    let cert_filename = out_cert_remote
        .file_name()
        .ok_or_else(|| TaskModuleError::Message("invalid out_cert".to_string()))?
        .to_str()
        .ok_or_else(|| TaskModuleError::Message("invalid out_cert string".to_string()))?;
    let pki_dir = host_vars
        .get("pki_dir")
        .and_then(Value::as_str)
        .unwrap_or("./pki");
    let pki_path = Path::new(pki_dir);
    let (out_key_local, out_cert_local) = if is_ca {
        (pki_path.join(key_filename), pki_path.join(cert_filename))
    } else {
        (
            pki_path.join(format!("{hostname}_{key_filename}")),
            pki_path.join(format!("{hostname}_{cert_filename}")),
        )
    };
    let root_key_local = rendered
        .get("root_key")
        .and_then(Value::as_str)
        .map(|s| pki_path.join(Path::new(s).file_name().unwrap_or_default()));
    let root_cert_local = rendered
        .get("root_cert")
        .and_then(Value::as_str)
        .map(|s| pki_path.join(Path::new(s).file_name().unwrap_or_default()));
    let sans: Vec<String> = rendered
        .get("sans")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect();
    let date_hours = rendered
        .get("date_hours")
        .and_then(Value::as_u64)
        .unwrap_or(24 * 365);
    let policy_str = rendered
        .get("policy")
        .and_then(Value::as_str)
        .unwrap_or("always");
    let policy = match policy_str {
        "ifnotpresent" | "if_not_present" => CertPolicy::IfNotPresent,
        "none" => CertPolicy::None,
        _ => CertPolicy::Always,
    };
    Ok(Some(GenCertParams {
        args: GenCertArgs {
            cn,
            out_key: out_key_local.clone(),
            out_cert: out_cert_local.clone(),
            root_key: root_key_local,
            root_cert: root_cert_local,
            sans,
            is_ca,
            date_hours,
            policy,
        },
        out_key_remote,
        out_cert_remote,
        out_key_local,
        out_cert_local,
    }))
}
async fn upload_cert_files(
    connector: &Connector,
    out_key_local: &Path,
    out_cert_local: &Path,
    out_key_remote: &Path,
    out_cert_remote: &Path,
) -> Result<(), TaskModuleError> {
    let key_bytes = tokio::fs::read(out_key_local).await.map_err(|e| {
        TaskModuleError::Message(format!(
            "failed to read generated key from {}: {e}",
            out_key_local.display()
        ))
    })?;
    let cert_bytes = tokio::fs::read(out_cert_local).await.map_err(|e| {
        TaskModuleError::Message(format!(
            "failed to read generated cert from {}: {e}",
            out_cert_local.display()
        ))
    })?;
    if let Some(parent) = out_key_remote.parent() {
        mkdir_p(connector, &parent.to_string_lossy())
            .await
            .map_err(TaskModuleError::from_connector)?;
    }
    atomic_put_bytes(
        connector,
        &key_bytes,
        &out_key_remote.to_string_lossy(),
        0o600,
    )
    .await
    .map_err(TaskModuleError::from_connector)?;
    atomic_put_bytes(
        connector,
        &cert_bytes,
        &out_cert_remote.to_string_lossy(),
        0o644,
    )
    .await
    .map_err(TaskModuleError::from_connector)?;
    Ok(())
}
async fn generate_and_measure(params: &GenCertParams) -> Result<bool, TaskModuleError> {
    let before_key = file_sha_or_none(&params.out_key_local);
    let before_cert = file_sha_or_none(&params.out_cert_local);
    let args_clone = params.args.clone();
    tokio::task::spawn_blocking(move || generate_certificate(&args_clone))
        .await
        .map_err(|e| TaskModuleError::Message(format!("gen_cert blocking task join failed: {e}")))?
        .map_err(|e| TaskModuleError::Message(format!("gen_cert failed: {e}")))?;
    let after_key = file_sha_or_none(&params.out_key_local);
    let after_cert = file_sha_or_none(&params.out_cert_local);
    Ok(before_key != after_key || before_cert != after_cert)
}
pub(crate) async fn execute_gen_cert(
    task: &Task,
    connector: &Connector,
    host_vars: &Value,
    context: &gtmpl::Value,
) -> Result<Option<TaskResult>, TaskModuleError> {
    let Some(params) = parse_gen_cert_params(task, host_vars, context)? else {
        return Ok(None);
    };
    if let Some(parent) = params.out_key_local.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| TaskModuleError::Message(format!("failed to create local pki dir: {e}")))?;
    }
    let content_changed = if params.args.is_ca {
        with_ca_lock(&params.out_key_local, generate_and_measure(&params)).await?
    } else {
        generate_and_measure(&params).await?
    };
    let local_key_bytes = tokio::fs::read(&params.out_key_local)
        .await
        .map_err(|e| TaskModuleError::Message(format!("failed to read generated key: {e}")))?;
    let local_cert_bytes = tokio::fs::read(&params.out_cert_local)
        .await
        .map_err(|e| TaskModuleError::Message(format!("failed to read generated cert: {e}")))?;
    let mut key_hasher = sha2::Sha256::new();
    key_hasher.update(&local_key_bytes);
    let local_key_sha = format!("{:x}", key_hasher.finalize());
    let mut cert_hasher = sha2::Sha256::new();
    cert_hasher.update(&local_cert_bytes);
    let local_cert_sha = format!("{:x}", cert_hasher.finalize());
    let key_q = shell_single_quote(&params.out_key_remote.to_string_lossy());
    let cert_q = shell_single_quote(&params.out_cert_remote.to_string_lossy());
    let check_remote_cmd = format!("sha256sum {key_q} {cert_q} 2>/dev/null");
    let check_res = connector
        .execute_shell(&check_remote_cmd)
        .await
        .map_err(TaskModuleError::from_connector)?;
    let mut remote_identical = false;
    if check_res.exit_code == 0 {
        let out_str = String::from_utf8_lossy(&check_res.stdout);
        let mut lines = out_str.lines();
        if let Some(key_line) = lines.next() {
            if let Some(cert_line) = lines.next() {
                if let Some(remote_key_sha) = key_line.split_whitespace().next() {
                    if let Some(remote_cert_sha) = cert_line.split_whitespace().next() {
                        if remote_key_sha.trim() == local_key_sha.trim()
                            && remote_cert_sha.trim() == local_cert_sha.trim()
                        {
                            remote_identical = true;
                        }
                    }
                }
            }
        }
    }
    if remote_identical && !content_changed {
        return Ok(Some(TaskResult {
            stdout: "cert files are already present and identical on remote".to_string(),
            stderr: String::new(),
            exit_code: 0,
            changed: false,
            failed: false,
        }));
    }
    upload_cert_files(
        connector,
        &params.out_key_local,
        &params.out_cert_local,
        &params.out_key_remote,
        &params.out_cert_remote,
    )
    .await?;
    Ok(Some(TaskResult {
        stdout: "cert generated and uploaded successfully".to_string(),
        stderr: String::new(),
        exit_code: 0,
        changed: true,
        failed: false,
    }))
}
