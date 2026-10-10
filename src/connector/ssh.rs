use super::{CommandResult, ConnectorError, SshConfig};
use ssh2_tokio::AsyncSession;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
pub(crate) struct SshConnector {
    pub config: SshConfig,
    pub host: String,
    pub session: Option<tokio::sync::Mutex<AsyncSession<tokio::net::TcpStream>>>,
}
fn default_identity_paths() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let base = Path::new(&home).join(".ssh");
    vec![
        base.join("id_ed25519"),
        base.join("id_ecdsa"),
        base.join("id_rsa"),
    ]
}
fn max_cmd_output_bytes() -> usize {
    const DEFAULT: usize = 16 * 1024 * 1024;
    match std::env::var("RTKK_MAX_CMD_OUTPUT_BYTES") {
        Ok(v) => v.trim().parse::<usize>().unwrap_or(DEFAULT).max(1024),
        Err(_) => DEFAULT,
    }
}
async fn try_auth_pubkey_memory(
    session: &mut AsyncSession<tokio::net::TcpStream>,
    username: &str,
    pkey_content: &str,
) -> Result<bool, ConnectorError> {
    if pkey_content.trim().is_empty() {
        return Ok(false);
    }
    session
        .userauth_pubkey_memory(username, None, pkey_content, None)
        .await
        .map_err(|e| {
            ConnectorError::AuthenticationFailed(format!("PrivateKeyContent auth failed: {e}"))
        })?;
    Ok(true)
}
async fn try_auth_pubkey_path(
    session: &mut AsyncSession<tokio::net::TcpStream>,
    username: &str,
    pkey_path: &str,
    fallback_password: bool,
) -> Result<(bool, bool), ConnectorError> {
    if pkey_path.trim().is_empty() {
        return Ok((false, false));
    }
    let path = Path::new(pkey_path);
    if !path.exists() {
        return Ok((false, false));
    }
    match session
        .userauth_pubkey_file(username, None, path, None)
        .await
    {
        Ok(()) => Ok((true, false)),
        Err(e) => {
            if fallback_password {
                tracing::warn!(
                    "PrivateKey path auth failed for {}, will try password fallback: {e}",
                    path.display()
                );
                Ok((false, true))
            } else {
                Err(ConnectorError::AuthenticationFailed(format!(
                    "PrivateKey path auth failed: {e}"
                )))
            }
        }
    }
}
async fn try_default_identities(
    session: &mut AsyncSession<tokio::net::TcpStream>,
    username: &str,
) -> bool {
    for default_key in default_identity_paths() {
        if default_key.exists()
            && session
                .userauth_pubkey_file(username, None, &default_key, None)
                .await
                .is_ok()
        {
            return true;
        }
    }
    false
}
async fn authenticate_session(
    session: &mut AsyncSession<tokio::net::TcpStream>,
    config: &SshConfig,
) -> Result<(), ConnectorError> {
    let mut authenticated = false;
    let mut configured_key_hard_failed = false;
    if let Some(ref pkey_content) = config.private_key_content {
        if try_auth_pubkey_memory(session, &config.username, pkey_content).await? {
            authenticated = true;
        }
    }
    if !authenticated {
        if let Some(ref pkey_path) = config.private_key {
            let (ok, hard_failed) = try_auth_pubkey_path(
                session,
                &config.username,
                pkey_path,
                config.private_key_fallback_password,
            )
            .await?;
            authenticated = ok;
            configured_key_hard_failed = hard_failed;
        }
    }
    let explicit_key_configured = config
        .private_key_content
        .as_ref()
        .is_some_and(|s| !s.trim().is_empty())
        || config
            .private_key
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty());
    if !authenticated && !explicit_key_configured {
        authenticated = try_default_identities(session, &config.username).await;
    }
    if !authenticated {
        if let Some(ref pass) = config.password {
            if configured_key_hard_failed && !config.private_key_fallback_password {
                return Err(ConnectorError::AuthenticationFailed(
                    "Private key authentication failed".to_string(),
                ));
            }
            session
                .userauth_password(&config.username, pass)
                .await
                .map_err(|e| {
                    ConnectorError::AuthenticationFailed(format!("Password auth failed: {e}"))
                })?;
        } else {
            return Err(ConnectorError::AuthenticationFailed(
                "No valid authentication credentials found".to_string(),
            ));
        }
    }
    Ok(())
}
impl SshConnector {
    pub(crate) async fn init(&mut self) -> Result<(), ConnectorError> {
        let port = self.config.port.unwrap_or(22);
        let addr = format!("{host}:{port}", host = self.host);
        let stream = tokio::net::TcpStream::connect(&addr).await.map_err(|e| {
            ConnectorError::ConnectionFailed(format!("TCP connection to {addr} failed: {e}"))
        })?;
        let mut session_config = ssh2_tokio::SessionConfiguration::new();
        session_config.set_keepalive(true, 30);
        let mut session = AsyncSession::new(stream, Some(session_config)).map_err(|e| {
            ConnectorError::ConnectionFailed(format!("Failed to create SSH session: {e}"))
        })?;
        session
            .handshake()
            .await
            .map_err(|e| ConnectorError::ConnectionFailed(format!("SSH Handshake failed: {e}")))?;
        authenticate_session(&mut session, &self.config).await?;
        self.session = Some(tokio::sync::Mutex::new(session));
        Ok(())
    }
    pub(crate) async fn execute_command(&self, cmd: &str) -> Result<CommandResult, ConnectorError> {
        let session_mutex = self.session.as_ref().ok_or_else(|| {
            ConnectorError::CommandExecutionFailed("SSH session not initialized".to_string())
        })?;
        let session = session_mutex.lock().await;
        let mut channel = session
            .channel_session()
            .await
            .map_err(|e| ConnectorError::CommandExecutionFailed(e.to_string()))?;
        channel
            .exec(cmd)
            .await
            .map_err(|e| ConnectorError::CommandExecutionFailed(e.to_string()))?;
        let max = max_cmd_output_bytes();
        let (stdout, stderr) = {
            let mut stdout_buf = Vec::new();
            let mut stderr_buf = Vec::new();
            let mut out_done = false;
            let mut err_done = false;
            let poll_timeout = std::time::Duration::from_millis(50);
            while !out_done || !err_done {
                if !out_done {
                    let mut out_chunk = [0u8; 8192];
                    match tokio::time::timeout(poll_timeout, channel.read(&mut out_chunk)).await {
                        Ok(Ok(0)) => out_done = true,
                        Ok(Ok(n)) => {
                            if stdout_buf.len().saturating_add(n) > max {
                                return Err(ConnectorError::IOError(format!(
                                    "command stdout exceeded RTKK_MAX_CMD_OUTPUT_BYTES ({max})"
                                )));
                            }
                            if let Some(slice) = out_chunk.get(..n) {
                                stdout_buf.extend_from_slice(slice);
                            }
                        }
                        Ok(Err(e)) => {
                            return Err(ConnectorError::IOError(e.to_string()));
                        }
                        Err(_) => {}
                    }
                }
                if !err_done {
                    let mut err_chunk = [0u8; 8192];
                    let mut stderr_reader = channel.stderr();
                    match tokio::time::timeout(poll_timeout, stderr_reader.read(&mut err_chunk))
                        .await
                    {
                        Ok(Ok(0)) => err_done = true,
                        Ok(Ok(n)) => {
                            if stderr_buf.len().saturating_add(n) > max {
                                return Err(ConnectorError::IOError(format!(
                                    "command stderr exceeded RTKK_MAX_CMD_OUTPUT_BYTES ({max})"
                                )));
                            }
                            if let Some(slice) = err_chunk.get(..n) {
                                stderr_buf.extend_from_slice(slice);
                            }
                        }
                        Ok(Err(e)) => {
                            return Err(ConnectorError::IOError(e.to_string()));
                        }
                        Err(_) => {}
                    }
                }
            }
            (stdout_buf, stderr_buf)
        };
        channel
            .wait_close()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        let exit_code = channel
            .exit_status()
            .map_err(|e| ConnectorError::CommandExecutionFailed(e.to_string()))?;
        Ok(CommandResult {
            stdout,
            stderr,
            exit_code,
        })
    }
    pub(crate) async fn execute_argv(&self, argv: &[&str]) -> Result<CommandResult, ConnectorError> {
        if argv.is_empty() {
            return Err(ConnectorError::CommandExecutionFailed(
                "empty argv".to_string(),
            ));
        }
        let joined = shell_words::join(argv);
        self.execute_command(&joined).await
    }
    pub(crate) async fn execute_shell(&self, script: &str) -> Result<CommandResult, ConnectorError> {
        let quoted = crate::utils::shell_single_quote(script);
        let cmd = format!("/bin/sh -c {quoted}");
        self.execute_command(&cmd).await
    }
    pub(crate) async fn put_file(
        &self,
        src: &[u8],
        dest: &Path,
        mode: u32,
    ) -> Result<(), ConnectorError> {
        let session_mutex = self
            .session
            .as_ref()
            .ok_or_else(|| ConnectorError::IOError("SSH session not initialized".to_string()))?;
        let session = session_mutex.lock().await;
        let sftp = session
            .sftp()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        let mut file = sftp
            .create(dest)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        futures_util::io::AsyncWriteExt::write_all(&mut file, src)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        drop(file);
        let mut stat = sftp
            .stat(dest)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        stat.perm = Some(mode);
        sftp.setstat(dest, stat)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        Ok(())
    }
    pub(crate) async fn fetch_file(&self, src: &Path, dest: &Path) -> Result<(), ConnectorError> {
        let session_mutex = self
            .session
            .as_ref()
            .ok_or_else(|| ConnectorError::IOError("SSH session not initialized".to_string()))?;
        let session = session_mutex.lock().await;
        let sftp = session
            .sftp()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        let mut remote_file = sftp
            .open(src)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        }
        let mut local_file = tokio::fs::File::create(dest).await.map_err(|e| {
            ConnectorError::IOError(format!(
                "Failed to create local file {dest}: {e}",
                dest = dest.display()
            ))
        })?;
        let mut buffer = vec![0u8; 65536];
        loop {
            let n = futures_util::io::AsyncReadExt::read(&mut remote_file, &mut buffer)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
            if n == 0 {
                break;
            }
            if let Some(slice) = buffer.get(..n) {
                local_file
                    .write_all(slice)
                    .await
                    .map_err(|e| ConnectorError::IOError(e.to_string()))?;
            } else {
                return Err(ConnectorError::IOError("Slicing out of bounds".to_string()));
            }
        }
        local_file
            .flush()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        Ok(())
    }
    pub(crate) async fn put_file_from_local(
        &self,
        local_path: &Path,
        dest: &Path,
        mode: u32,
    ) -> Result<(), ConnectorError> {
        let session_mutex = self
            .session
            .as_ref()
            .ok_or_else(|| ConnectorError::IOError("SSH session not initialized".to_string()))?;
        let session = session_mutex.lock().await;
        let sftp = session
            .sftp()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        let mut remote_file = sftp
            .create(dest)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        let mut local_file = tokio::fs::File::open(local_path).await.map_err(|e| {
            ConnectorError::IOError(format!(
                "Failed to open local file {local_path}: {e}",
                local_path = local_path.display()
            ))
        })?;
        let mut buffer = vec![0u8; 65536];
        loop {
            let n = local_file
                .read(&mut buffer)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
            if n == 0 {
                break;
            }
            if let Some(slice) = buffer.get(..n) {
                futures_util::io::AsyncWriteExt::write_all(&mut remote_file, slice)
                    .await
                    .map_err(|e| ConnectorError::IOError(e.to_string()))?;
            } else {
                return Err(ConnectorError::IOError("Slicing out of bounds".to_string()));
            }
        }
        drop(remote_file);
        let mut stat = sftp
            .stat(dest)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        stat.perm = Some(mode);
        sftp.setstat(dest, stat)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        Ok(())
    }
    pub(crate) async fn close(&self) -> Result<(), ConnectorError> {
        if let Some(ref session_mutex) = self.session {
            let session = session_mutex.lock().await;
            let _ = session
                .disconnect(None, "Playbook run finished", None)
                .await;
        }
        Ok(())
    }
}
