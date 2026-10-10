use crate::connector::{CommandResult, Connector, ConnectorError};
use crate::utils::shell_single_quote;
use std::path::Path;
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
    let tmp = format!("{dest}.tmp");
    let put_res = connector.put_file(data, Path::new(&tmp), mode).await;
    if let Err(e) = put_res {
        let _ = cleanup_tmp(connector, &tmp).await;
        return Err(e);
    }
    finish_atomic_mv(connector, &tmp, dest).await
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
    let tmp = format!("{dest}.tmp");
    let put_res = connector
        .put_file_from_local(local_path, Path::new(&tmp), mode)
        .await;
    if let Err(e) = put_res {
        let _ = cleanup_tmp(connector, &tmp).await;
        return Err(e);
    }
    finish_atomic_mv(connector, &tmp, dest).await
}
async fn finish_atomic_mv(
    connector: &Connector,
    tmp: &str,
    dest: &str,
) -> Result<(), ConnectorError> {
    let mv_cmd = format!(
        "mv -f {tmp_q} {dest_q}",
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
