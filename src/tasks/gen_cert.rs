use super::copy::{RemoteCheckOutcome, check_remote_sha_and_mode};
use super::remote_fs::{atomic_put_bytes, chmod_remote, mkdir_p};
use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::cert::{
    CertPolicy, DEFAULT_CERT_VALIDITY_HOURS, GenCertArgs, generate_certificate,
    parse_duration_value,
};
use crate::connector::Connector;
use serde_json::Value;
use sha2::Digest as _;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex as TokioMutex;

const KEY_FILE_MODE: u32 = 0o600;
const CERT_FILE_MODE: u32 = 0o644;

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
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
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

fn extract_filename(path_str: &str, label: &str) -> Result<String, TaskModuleError> {
    if path_str.trim().is_empty() {
        return Err(TaskModuleError::Message(format!(
            "gen_cert: {label} cannot be empty"
        )));
    }
    Path::new(path_str)
        .file_name()
        .and_then(|f| f.to_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| TaskModuleError::Message(format!("gen_cert: invalid {label} '{path_str}'")))
}

fn parse_root_ca_paths(
    rendered: &Value,
    pki_path: &Path,
) -> Result<(Option<PathBuf>, Option<PathBuf>), TaskModuleError> {
    let root_key_opt = match rendered.get("root_key") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(pki_path.join(extract_filename(s, "root_key")?)),
        Some(_) => {
            return Err(TaskModuleError::Message(
                "gen_cert: root_key must be a string".to_string(),
            ));
        }
    };
    let root_cert_opt = match rendered.get("root_cert") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(pki_path.join(extract_filename(s, "root_cert")?)),
        Some(_) => {
            return Err(TaskModuleError::Message(
                "gen_cert: root_cert must be a string".to_string(),
            ));
        }
    };
    if root_key_opt.is_some() != root_cert_opt.is_some() {
        return Err(TaskModuleError::Message(
            "gen_cert: root_key and root_cert must both be specified or both omitted".to_string(),
        ));
    }
    Ok((root_key_opt, root_cert_opt))
}

fn resolve_validity_hours(rendered: &Value, host_vars: &Value) -> Result<u64, TaskModuleError> {
    let task_dur = rendered
        .get("date_hours")
        .filter(|v| !v.is_null())
        .or_else(|| rendered.get("date").filter(|v| !v.is_null()));
    if let Some(val) = task_dur {
        return parse_duration_value(val).map_err(TaskModuleError::Message);
    }
    if let Some(val) = host_vars.get("cert_validity").filter(|v| !v.is_null()) {
        return parse_duration_value(val).map_err(TaskModuleError::Message);
    }
    Ok(DEFAULT_CERT_VALIDITY_HOURS)
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
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| TaskModuleError::Message("gen_cert: missing cn".to_string()))?
        .to_string();
    let out_key_str = rendered
        .get("out_key")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("gen_cert: missing out_key".to_string()))?;
    let out_cert_str = rendered
        .get("out_cert")
        .and_then(Value::as_str)
        .ok_or_else(|| TaskModuleError::Message("gen_cert: missing out_cert".to_string()))?;
    let key_filename = extract_filename(out_key_str, "out_key")?;
    let cert_filename = extract_filename(out_cert_str, "out_cert")?;
    let out_key_remote = PathBuf::from(out_key_str);
    let out_cert_remote = PathBuf::from(out_cert_str);

    let hostname = host_vars
        .get("inventory_hostname")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("local");
    let is_ca = rendered
        .get("is_ca")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let pki_dir = host_vars
        .get("pki_dir")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
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
    let (root_key, root_cert) = parse_root_ca_paths(&rendered, pki_path)?;
    let sans: Vec<String> = rendered
        .get("sans")
        .and_then(Value::as_array)
        .map_or_default(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        });
    let date_hours = resolve_validity_hours(&rendered, host_vars)?;
    let policy_str = rendered
        .get("policy")
        .or_else(|| rendered.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("always");
    let policy = CertPolicy::from_str_policy(policy_str).map_err(TaskModuleError::Message)?;

    Ok(Some(GenCertParams {
        args: GenCertArgs {
            cn,
            out_key: out_key_local.clone(),
            out_cert: out_cert_local.clone(),
            root_key,
            root_cert,
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
    key_bytes: &[u8],
    cert_bytes: &[u8],
    out_key_remote: &Path,
    out_cert_remote: &Path,
) -> Result<(), TaskModuleError> {
    if let Some(parent) = out_key_remote.parent() {
        mkdir_p(connector, &parent.to_string_lossy())
            .await
            .map_err(TaskModuleError::from_connector)?;
    }
    if let Some(parent) = out_cert_remote.parent()
        && out_key_remote.parent() != Some(parent)
    {
        mkdir_p(connector, &parent.to_string_lossy())
            .await
            .map_err(TaskModuleError::from_connector)?;
    }
    atomic_put_bytes(
        connector,
        key_bytes,
        &out_key_remote.to_string_lossy(),
        KEY_FILE_MODE,
    )
    .await
    .map_err(TaskModuleError::from_connector)?;
    atomic_put_bytes(
        connector,
        cert_bytes,
        &out_cert_remote.to_string_lossy(),
        CERT_FILE_MODE,
    )
    .await
    .map_err(TaskModuleError::from_connector)?;
    Ok(())
}

async fn generate_and_measure(params: &GenCertParams) -> Result<bool, TaskModuleError> {
    let before_key = file_sha_or_none(&params.out_key_local);
    let before_cert = file_sha_or_none(&params.out_cert_local);
    let args_clone = params.args.clone();
    let generated = tokio::task::spawn_blocking(move || generate_certificate(&args_clone))
        .await
        .map_err(|e| TaskModuleError::Message(format!("gen_cert blocking task join failed: {e}")))?
        .map_err(|e| TaskModuleError::Message(format!("gen_cert failed: {e}")))?;
    let after_key = file_sha_or_none(&params.out_key_local);
    let after_cert = file_sha_or_none(&params.out_cert_local);
    Ok(generated || before_key != after_key || before_cert != after_cert)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
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
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            TaskModuleError::Message(format!("failed to create local pki dir: {e}"))
        })?;
    }
    if let Some(parent) = params.out_cert_local.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            TaskModuleError::Message(format!("failed to create local pki dir: {e}"))
        })?;
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
    let local_key_sha = sha256_hex(&local_key_bytes);
    let local_cert_sha = sha256_hex(&local_cert_bytes);

    let key_remote_str = params.out_key_remote.to_string_lossy().into_owned();
    let cert_remote_str = params.out_cert_remote.to_string_lossy().into_owned();
    let key_check =
        check_remote_sha_and_mode(connector, &key_remote_str, &local_key_sha, KEY_FILE_MODE)
            .await?;
    let cert_check =
        check_remote_sha_and_mode(connector, &cert_remote_str, &local_cert_sha, CERT_FILE_MODE)
            .await?;

    if !content_changed
        && matches!(key_check, RemoteCheckOutcome::Identical)
        && matches!(cert_check, RemoteCheckOutcome::Identical)
    {
        return Ok(Some(TaskResult {
            stdout: "cert files are already present and identical on remote".to_string(),
            stderr: String::new(),
            exit_code: 0,
            changed: false,
            failed: false,
        }));
    }

    if !content_changed
        && !matches!(key_check, RemoteCheckOutcome::ContentDiffersOrMissing)
        && !matches!(cert_check, RemoteCheckOutcome::ContentDiffersOrMissing)
    {
        if matches!(key_check, RemoteCheckOutcome::ModeMismatch) {
            chmod_remote(connector, &key_remote_str, KEY_FILE_MODE)
                .await
                .map_err(TaskModuleError::from_connector)?;
        }
        if matches!(cert_check, RemoteCheckOutcome::ModeMismatch) {
            chmod_remote(connector, &cert_remote_str, CERT_FILE_MODE)
                .await
                .map_err(TaskModuleError::from_connector)?;
        }
        return Ok(Some(TaskResult {
            stdout: "cert file permissions updated on remote".to_string(),
            stderr: String::new(),
            exit_code: 0,
            changed: true,
            failed: false,
        }));
    }

    upload_cert_files(
        connector,
        &local_key_bytes,
        &local_cert_bytes,
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
