use crate::connector::{CommandResult, Connector, ConnectorError, unique_tmp_id};
use crate::utils::shell_single_quote;
use std::path::Path;

pub(crate) fn unique_remote_tmp(dest: &str) -> String {
    let (pid, rand_u64) = unique_tmp_id();
    format!("{dest}.rtkk.tmp.{pid}.{rand_u64}")
}

pub(crate) fn octal_modes_equal(remote_octal: &str, expected_mode: u32) -> bool {
    let local_octal = format!("{:o}", expected_mode & 0o7777);
    let remote_norm = remote_octal.trim().trim_start_matches('0');
    let local_norm = local_octal.trim_start_matches('0');
    remote_norm == local_norm || (remote_norm.is_empty() && local_norm.is_empty())
}

pub(crate) async fn chmod_remote(
    connector: &Connector,
    dest: &str,
    mode: u32,
) -> Result<(), ConnectorError> {
    let perm = mode & 0o7777;
    let cmd = format!("chmod {perm:o} {q}", q = shell_single_quote(dest));
    let res = connector.execute_shell(&cmd).await?;
    if res.exit_code != 0 {
        let stderr = String::from_utf8_lossy(&res.stderr);
        return Err(ConnectorError::CommandExecutionFailed(format!(
            "chmod {perm:o} {dest} failed (rc={}): {stderr}",
            res.exit_code
        )));
    }
    Ok(())
}

pub(crate) async fn mkdir_p(connector: &Connector, dir: &str) -> Result<(), ConnectorError> {
    if dir.is_empty() || dir == "." || dir == "/" {
        return Ok(());
    }
    let cmd = format!("mkdir -p {q}", q = shell_single_quote(dir));
    let res = connector.execute_shell(&cmd).await?;
    if res.exit_code != 0 {
        let stderr = String::from_utf8_lossy(&res.stderr);
        return Err(ConnectorError::CommandExecutionFailed(format!(
            "mkdir -p {dir} failed (rc={}): {stderr}",
            res.exit_code
        )));
    }
    Ok(())
}

pub(crate) async fn atomic_put_bytes(
    connector: &Connector,
    data: &[u8],
    dest: &str,
    mode: u32,
) -> Result<(), ConnectorError> {
    let dest_path = Path::new(dest);
    let parent = dest_path
        .parent()
        .map_or_else(|| ".".to_string(), |p| p.to_string_lossy().into_owned());
    mkdir_p(connector, &parent).await?;
    let tmp = unique_remote_tmp(dest);
    let put_res = connector.put_file(data, Path::new(&tmp), mode).await;
    if let Err(e) = put_res {
        let _ = cleanup_tmp(connector, &tmp).await;
        return Err(e);
    }
    finish_atomic_mv(connector, &tmp, dest, mode).await
}

pub(crate) async fn atomic_put_local_path(
    connector: &Connector,
    local_path: &Path,
    dest: &str,
    mode: u32,
) -> Result<(), ConnectorError> {
    let dest_path = Path::new(dest);
    let parent = dest_path
        .parent()
        .map_or_else(|| ".".to_string(), |p| p.to_string_lossy().into_owned());
    mkdir_p(connector, &parent).await?;
    let tmp = unique_remote_tmp(dest);
    let put_res = connector
        .put_file_from_local(local_path, Path::new(&tmp), mode)
        .await;
    if let Err(e) = put_res {
        let _ = cleanup_tmp(connector, &tmp).await;
        return Err(e);
    }
    finish_atomic_mv(connector, &tmp, dest, mode).await
}

async fn finish_atomic_mv(
    connector: &Connector,
    tmp: &str,
    dest: &str,
    mode: u32,
) -> Result<(), ConnectorError> {
    let perm = mode & 0o7777;
    let mv_cmd = format!(
        "chmod {perm:o} {tmp_q} && mv -f {tmp_q} {dest_q}",
        tmp_q = shell_single_quote(tmp),
        dest_q = shell_single_quote(dest),
    );
    let res = connector.execute_shell(&mv_cmd).await;
    match res {
        Ok(r) if r.exit_code == 0 => Ok(()),
        Ok(r) => {
            let _ = cleanup_tmp(connector, tmp).await;
            let stderr = String::from_utf8_lossy(&r.stderr);
            Err(ConnectorError::CommandExecutionFailed(format!(
                "atomic mv {tmp} -> {dest} failed (rc={}): {stderr}",
                r.exit_code
            )))
        }
        Err(e) => {
            let _ = cleanup_tmp(connector, tmp).await;
            Err(e)
        }
    }
}

async fn cleanup_tmp(connector: &Connector, tmp: &str) -> Result<CommandResult, ConnectorError> {
    let cmd = format!("rm -f {q}", q = shell_single_quote(tmp));
    connector.execute_shell(&cmd).await
}
