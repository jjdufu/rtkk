use super::{
    BecomeMethod, CommandResult, ConnectorError, HostKeyPolicy, PrivilegeConfig, SshConfig,
    build_su_shell_cmd, build_sudo_shell_cmd, contains_password_prompt, contains_subslice,
    format_host_port, max_cmd_output_bytes, parse_su_framed_output, read_stream_bounded,
    strip_sudo_prompt, unique_tmp_id,
};
use crate::utils::shell_single_quote;
use ssh2_tokio::AsyncSession;
use ssh2_tokio::ssh2::{
    CheckResult, KnownHostFileKind, KnownHostKeyFormat, PtyModeOpcode, PtyModes,
};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// Connector executing commands and file transfers over `SSH` and `SFTP`.
pub struct SshConnector {
    /// `SSH` transport, credential, and host-key settings.
    pub config: SshConfig,
    /// Target hostname, `IPv4`, or `IPv6` address.
    pub host: String,
    /// Optional privilege escalation (`sudo` / `become`) configuration.
    pub privilege: Option<PrivilegeConfig>,
    /// Active asynchronous `SSH` session guarded by a mutex.
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

fn resolve_known_hosts_path(custom: Option<&str>) -> PathBuf {
    if let Some(raw) = custom.map(str::trim).filter(|s| !s.is_empty()) {
        return crate::utils::expand_tilde_path(raw);
    }
    let home = std::env::var("HOME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/root".to_string());
    Path::new(&home).join(".ssh").join("known_hosts")
}

fn verify_host_key_sync(
    session: &AsyncSession<tokio::net::TcpStream>,
    host: &str,
    port: u16,
    policy: HostKeyPolicy,
    known_hosts_path: &Path,
) -> Result<(), ConnectorError> {
    if policy == HostKeyPolicy::Off {
        return Ok(());
    }
    let clean_host = host
        .trim()
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or_else(|| host.trim());
    if clean_host.is_empty() || clean_host.contains('\0') {
        return Err(ConnectorError::AuthenticationFailed(
            "Invalid host name for SSH host key verification".to_string(),
        ));
    }
    let (key_bytes, key_type) = session.host_key().ok_or_else(|| {
        ConnectorError::AuthenticationFailed(format!(
            "SSH server {clean_host}:{port} did not present a host key"
        ))
    })?;
    let mut known_hosts = session.known_hosts().map_err(|e| {
        ConnectorError::AuthenticationFailed(format!("Failed to initialize known_hosts: {e}"))
    })?;
    if known_hosts_path.exists() {
        known_hosts
            .read_file(known_hosts_path, KnownHostFileKind::OpenSSH)
            .map_err(|e| {
                ConnectorError::AuthenticationFailed(format!(
                    "Failed to read known_hosts file {}: {e}",
                    known_hosts_path.display()
                ))
            })?;
    }
    match known_hosts.check_port(clean_host, port, key_bytes) {
        CheckResult::Match => Ok(()),
        CheckResult::Mismatch => Err(ConnectorError::AuthenticationFailed(format!(
            "SSH host key mismatch for {clean_host}:{port} in {}",
            known_hosts_path.display()
        ))),
        CheckResult::Failure => Err(ConnectorError::AuthenticationFailed(format!(
            "SSH host key verification failed for {clean_host}:{port}"
        ))),
        CheckResult::NotFound => match policy {
            HostKeyPolicy::Strict => Err(ConnectorError::AuthenticationFailed(format!(
                "SSH host key for {clean_host}:{port} is unknown in {} (strict policy)",
                known_hosts_path.display()
            ))),
            HostKeyPolicy::AcceptNew => {
                let entry_host = if port == 22 {
                    clean_host.to_string()
                } else {
                    format!("[{clean_host}]:{port}")
                };
                known_hosts
                    .add(
                        &entry_host,
                        key_bytes,
                        "",
                        KnownHostKeyFormat::from(key_type),
                    )
                    .map_err(|e| {
                        ConnectorError::AuthenticationFailed(format!(
                            "Failed to add host key for {entry_host}: {e}"
                        ))
                    })?;
                known_hosts
                    .write_file(known_hosts_path, KnownHostFileKind::OpenSSH)
                    .map_err(|e| {
                        ConnectorError::IOError(format!(
                            "Failed to write known_hosts file {}: {e}",
                            known_hosts_path.display()
                        ))
                    })?;
                Ok(())
            }
            HostKeyPolicy::Off => Ok(()),
        },
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
    let trimmed = pkey_path.trim();
    if trimmed.is_empty() {
        return Ok((false, false));
    }
    let path = crate::utils::expand_tilde_path(trimmed);
    if !path.exists() {
        if fallback_password {
            tracing::warn!(
                "Configured SSH private key file {} (from '{trimmed}') not found, will try password fallback",
                path.display()
            );
            return Ok((false, true));
        }
        return Err(ConnectorError::AuthenticationFailed(format!(
            "Configured SSH private key file not found: {} (from '{trimmed}')",
            path.display()
        )));
    }
    match session
        .userauth_pubkey_file(username, None, &path, None)
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
                    "PrivateKey path auth failed for {}: {e}",
                    path.display()
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
    if let Some(ref pkey_content) = config.private_key_content
        && try_auth_pubkey_memory(session, &config.username, pkey_content).await?
    {
        authenticated = true;
    }
    if !authenticated && let Some(ref pkey_path) = config.private_key {
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
    fn active_escalation(&self) -> Option<(BecomeMethod, &str, Option<&str>)> {
        let priv_cfg = self.privilege.as_ref()?;
        if !priv_cfg.become_enabled {
            return None;
        }
        let target_user = priv_cfg
            .become_user
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("root");
        if self.config.username == target_user {
            return None;
        }
        Some((
            priv_cfg.become_method,
            target_user,
            priv_cfg.become_password.as_deref(),
        ))
    }

    /// Connects to the remote `SSH` server, verifies its host key, and authenticates.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if `TCP` connection, `SSH` handshake, host-key verification,
    /// or user authentication fails.
    pub async fn init(&mut self) -> Result<(), ConnectorError> {
        let port = self.config.port.unwrap_or(22);
        let addr = format_host_port(&self.host, port);
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
        if self.config.host_key_policy != HostKeyPolicy::Off {
            let kh_path = resolve_known_hosts_path(self.config.known_hosts_file.as_deref());
            if self.config.host_key_policy == HostKeyPolicy::AcceptNew
                && let Some(parent) = kh_path.parent().filter(|p| !p.as_os_str().is_empty())
            {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    ConnectorError::IOError(format!(
                        "Failed to create known_hosts directory {}: {e}",
                        parent.display()
                    ))
                })?;
            }
            verify_host_key_sync(
                &session,
                &self.host,
                port,
                self.config.host_key_policy,
                &kh_path,
            )?;
        }
        authenticate_session(&mut session, &self.config).await?;
        self.session = Some(tokio::sync::Mutex::new(session));
        Ok(())
    }

    async fn exec_raw(
        &self,
        cmd: &str,
        stdin_data: Option<&[u8]>,
        strip_prompt: bool,
    ) -> Result<CommandResult, ConnectorError> {
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

        let max_bytes = max_cmd_output_bytes();
        let mut stdout_stream = channel.stream(0);
        let mut stderr_stream = channel.stderr();
        let stdin_fut = async {
            if let Some(data) = stdin_data {
                let mut stdin_stream = channel.stream(0);
                if let Err(e) = stdin_stream.write_all(data).await
                    && e.kind() != std::io::ErrorKind::BrokenPipe
                {
                    return Err(ConnectorError::IOError(e.to_string()));
                }
                let _ = stdin_stream.flush().await;
                let _ = channel.send_eof().await;
            }
            Ok::<(), ConnectorError>(())
        };
        let stdout_fut = read_stream_bounded(&mut stdout_stream, max_bytes, "stdout");
        let stderr_fut = read_stream_bounded(&mut stderr_stream, max_bytes, "stderr");
        let ((), stdout, raw_stderr) = tokio::try_join!(stdin_fut, stdout_fut, stderr_fut)?;

        channel
            .wait_eof()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        channel
            .close()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        channel
            .wait_close()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        let exit_code = channel
            .exit_status()
            .map_err(|e| ConnectorError::CommandExecutionFailed(e.to_string()))?;
        let stderr = if strip_prompt {
            strip_sudo_prompt(raw_stderr)
        } else {
            raw_stderr
        };
        Ok(CommandResult {
            stdout,
            stderr,
            exit_code,
        })
    }

    async fn exec_su_pty(
        &self,
        target_user: &str,
        inner_cmd: &str,
        pw_opt: Option<&str>,
    ) -> Result<CommandResult, ConnectorError> {
        let pw = pw_opt.filter(|s| !s.is_empty()).ok_or_else(|| {
            ConnectorError::AuthenticationFailed(
                "become_method 'su' requires non-empty ansible_become_pass (target user password)"
                    .to_string(),
            )
        })?;
        let frame = build_su_shell_cmd(target_user, inner_cmd);
        let session_mutex = self.session.as_ref().ok_or_else(|| {
            ConnectorError::CommandExecutionFailed("SSH session not initialized".to_string())
        })?;
        let session = session_mutex.lock().await;
        let mut channel = session
            .channel_session()
            .await
            .map_err(|e| ConnectorError::CommandExecutionFailed(e.to_string()))?;

        let mut pty_modes = PtyModes::new();
        pty_modes.set_boolean(PtyModeOpcode::ECHO, false);
        pty_modes.set_boolean(PtyModeOpcode::ONLCR, false);
        pty_modes.set_boolean(PtyModeOpcode::OCRNL, false);
        channel
            .request_pty("dumb", Some(pty_modes), None)
            .await
            .map_err(|e| {
                ConnectorError::CommandExecutionFailed(format!("failed to request SSH PTY: {e}"))
            })?;
        channel
            .exec(&frame.cmd)
            .await
            .map_err(|e| ConnectorError::CommandExecutionFailed(e.to_string()))?;

        let max_bytes = max_cmd_output_bytes();
        let mut stream = channel.stream(0);
        let mut stderr_stream = channel.stderr();
        let mut raw_buf = Vec::new();
        let mut chunk = vec![0u8; 8192];
        let mut password_sent = false;
        let pw_line = format!("{pw}\n");
        let start_bytes = frame.start_marker.as_bytes();

        loop {
            let n = stream
                .read(&mut chunk)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
            if n == 0 {
                break;
            }
            if raw_buf.len().saturating_add(n) > max_bytes {
                return Err(ConnectorError::IOError(format!(
                    "command stdout exceeded RTKK_MAX_CMD_OUTPUT_BYTES ({max_bytes})"
                )));
            }
            if let Some(slice) = chunk.get(..n) {
                raw_buf.extend_from_slice(slice);
            }
            if !password_sent {
                if contains_subslice(&raw_buf, start_bytes) {
                    password_sent = true;
                } else if contains_password_prompt(&raw_buf) {
                    if let Err(e) = stream.write_all(pw_line.as_bytes()).await
                        && e.kind() != std::io::ErrorKind::BrokenPipe
                    {
                        return Err(ConnectorError::IOError(e.to_string()));
                    }
                    let _ = stream.flush().await;
                    password_sent = true;
                }
            }
        }

        let extra_stderr = read_stream_bounded(&mut stderr_stream, max_bytes, "stderr").await?;
        channel
            .wait_eof()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        channel
            .close()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        channel
            .wait_close()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        let exit_code = channel
            .exit_status()
            .map_err(|e| ConnectorError::CommandExecutionFailed(e.to_string()))?;

        let mut parsed =
            parse_su_framed_output(&raw_buf, &frame.start_marker, &frame.err_marker, exit_code)?;
        if !extra_stderr.is_empty() {
            if !parsed.stderr.is_empty() && !parsed.stderr.ends_with(b"\n") {
                parsed.stderr.push(b'\n');
            }
            parsed.stderr.extend_from_slice(&extra_stderr);
        }
        Ok(parsed)
    }

    /// Executes `cmd` on the remote `SSH` session, applying `sudo` or `su` escalation if configured.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if the `SSH` channel, privilege escalation, or stream I/O fails.
    pub async fn execute_command(&self, cmd: &str) -> Result<CommandResult, ConnectorError> {
        if let Some((method, target_user, pw_opt)) = self.active_escalation() {
            if method == BecomeMethod::Su {
                return self.exec_su_pty(target_user, cmd, pw_opt).await;
            }
            let wrapped = build_sudo_shell_cmd(target_user, cmd, pw_opt.is_some());
            if let Some(pw) = pw_opt {
                let stdin_buf = format!("{pw}\n");
                return self
                    .exec_raw(&wrapped, Some(stdin_buf.as_bytes()), true)
                    .await;
            }
            return self.exec_raw(&wrapped, None, false).await;
        }
        self.exec_raw(cmd, None, false).await
    }

    /// Joins `argv` using `POSIX` shell quoting and executes it over `SSH`.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if `argv` is empty or remote execution fails.
    pub async fn execute_argv(&self, argv: &[&str]) -> Result<CommandResult, ConnectorError> {
        if argv.is_empty() {
            return Err(ConnectorError::CommandExecutionFailed(
                "empty argv".to_string(),
            ));
        }
        let joined = shell_words::join(argv);
        self.execute_command(&joined).await
    }

    /// Executes `script` via `/bin/sh -c` on the remote host.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if remote execution or stream I/O fails.
    pub async fn execute_shell(&self, script: &str) -> Result<CommandResult, ConnectorError> {
        if let Some((method, target_user, pw_opt)) = self.active_escalation() {
            if method == BecomeMethod::Su {
                return self.exec_su_pty(target_user, script, pw_opt).await;
            }
            let wrapped = build_sudo_shell_cmd(target_user, script, pw_opt.is_some());
            if let Some(pw) = pw_opt {
                let stdin_buf = format!("{pw}\n");
                return self
                    .exec_raw(&wrapped, Some(stdin_buf.as_bytes()), true)
                    .await;
            }
            return self.exec_raw(&wrapped, None, false).await;
        }
        let quoted = shell_single_quote(script);
        let cmd = format!("/bin/sh -c {quoted}");
        self.exec_raw(&cmd, None, false).await
    }

    async fn sftp_put_bytes(
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
            .map_err(|e| ConnectorError::IOError(e.to_string()))
    }

    async fn sftp_put_local(
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
                "Failed to open local file {}: {e}",
                local_path.display()
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
            .map_err(|e| ConnectorError::IOError(e.to_string()))
    }

    async fn sftp_unlink_quiet(&self, path: &Path) {
        if let Some(ref session_mutex) = self.session {
            let session = session_mutex.lock().await;
            if let Ok(sftp) = session.sftp().await {
                let _ = sftp.unlink(path).await;
            }
        }
    }

    async fn install_staged_remote(
        &self,
        stage: &Path,
        dest: &Path,
        mode: u32,
    ) -> Result<(), ConnectorError> {
        let stage_q = shell_single_quote(&stage.to_string_lossy());
        let dest_q = shell_single_quote(&dest.to_string_lossy());
        let cmd = if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
            let parent_q = shell_single_quote(&parent.to_string_lossy());
            format!(
                "mkdir -p {parent_q} && install -m {mode:o} {stage_q} {dest_q} && rm -f {stage_q}"
            )
        } else {
            format!("install -m {mode:o} {stage_q} {dest_q} && rm -f {stage_q}")
        };
        let res = self.execute_shell(&cmd).await;
        match res {
            Ok(r) if r.exit_code == 0 => Ok(()),
            Ok(r) => {
                self.sftp_unlink_quiet(stage).await;
                let _ = self.execute_shell(&format!("rm -f {stage_q}")).await;
                Err(ConnectorError::IOError(format!(
                    "privileged install to {} failed (rc={}): {}",
                    dest.display(),
                    r.exit_code,
                    String::from_utf8_lossy(&r.stderr)
                )))
            }
            Err(e) => {
                self.sftp_unlink_quiet(stage).await;
                let _ = self.execute_shell(&format!("rm -f {stage_q}")).await;
                Err(e)
            }
        }
    }

    /// Uploads `src` bytes to `dest` on the remote host with permissions `mode`.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if `SFTP` upload or privileged installation fails.
    pub async fn put_file(&self, src: &[u8], dest: &Path, mode: u32) -> Result<(), ConnectorError> {
        if self.active_escalation().is_some() {
            let (pid, rand_id) = unique_tmp_id();
            let stage = PathBuf::from(format!("/tmp/.rtkk_stage_{pid}_{rand_id:016x}"));
            if let Err(e) = self.sftp_put_bytes(src, &stage, 0o600).await {
                self.sftp_unlink_quiet(&stage).await;
                return Err(e);
            }
            return self.install_staged_remote(&stage, dest, mode).await;
        }
        self.sftp_put_bytes(src, dest, mode).await
    }

    async fn fetch_via_sftp(&self, src: &Path, tmp_dest: &Path) -> Result<(), ConnectorError> {
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
        let mut local_file = tokio::fs::File::create(tmp_dest).await.map_err(|e| {
            ConnectorError::IOError(format!(
                "Failed to create local file {}: {e}",
                tmp_dest.display()
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
            }
        }
        local_file
            .flush()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))
    }

    async fn stream_to_local_file(
        stdout_stream: &mut ssh2_tokio::AsyncStream<tokio::net::TcpStream>,
        tmp_dest: &Path,
    ) -> Result<(), ConnectorError> {
        let mut local_file = tokio::fs::File::create(tmp_dest)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        let mut buf = vec![0u8; 65536];
        loop {
            let n = stdout_stream
                .read(&mut buf)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
            if n == 0 {
                break;
            }
            if let Some(slice) = buf.get(..n) {
                local_file
                    .write_all(slice)
                    .await
                    .map_err(|e| ConnectorError::IOError(e.to_string()))?;
            }
        }
        local_file
            .flush()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))
    }

    async fn fetch_via_privileged_cat(
        &self,
        src: &Path,
        tmp_dest: &Path,
        target_user: &str,
        pw_opt: Option<&str>,
    ) -> Result<(), ConnectorError> {
        let session_mutex = self
            .session
            .as_ref()
            .ok_or_else(|| ConnectorError::IOError("SSH session not initialized".to_string()))?;
        let session = session_mutex.lock().await;
        let mut channel = session
            .channel_session()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        let cat_cmd = format!("cat -- {}", shell_single_quote(&src.to_string_lossy()));
        let wrapped = build_sudo_shell_cmd(target_user, &cat_cmd, pw_opt.is_some());
        channel
            .exec(&wrapped)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;

        let mut stdout_stream = channel.stream(0);
        let mut stderr_stream = channel.stderr();
        let stdin_buf = pw_opt.map(|pw| format!("{pw}\n"));
        let stdin_fut = async {
            if let Some(ref data) = stdin_buf {
                let mut stdin_stream = channel.stream(0);
                if let Err(e) = stdin_stream.write_all(data.as_bytes()).await
                    && e.kind() != std::io::ErrorKind::BrokenPipe
                {
                    return Err(ConnectorError::IOError(e.to_string()));
                }
                let _ = stdin_stream.flush().await;
                let _ = channel.send_eof().await;
            }
            Ok::<(), ConnectorError>(())
        };
        let copy_fut = Self::stream_to_local_file(&mut stdout_stream, tmp_dest);
        let stderr_fut = read_stream_bounded(&mut stderr_stream, max_cmd_output_bytes(), "stderr");
        let ((), (), raw_stderr) = tokio::try_join!(stdin_fut, copy_fut, stderr_fut)?;
        let _ = channel.wait_eof().await;
        let _ = channel.close().await;
        channel
            .wait_close()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        let rc = channel
            .exit_status()
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        if rc != 0 {
            let err_bytes = if pw_opt.is_some() {
                strip_sudo_prompt(raw_stderr)
            } else {
                raw_stderr
            };
            return Err(ConnectorError::IOError(format!(
                "privileged cat {} failed (rc={rc}): {}",
                src.display(),
                String::from_utf8_lossy(&err_bytes)
            )));
        }
        Ok(())
    }

    async fn fetch_via_su_staged_sftp(
        &self,
        src: &Path,
        tmp_dest: &Path,
    ) -> Result<(), ConnectorError> {
        let (pid, rand_id) = unique_tmp_id();
        let stage = PathBuf::from(format!("/tmp/.rtkk_fetch_{pid}_{rand_id:016x}"));
        let src_q = shell_single_quote(&src.to_string_lossy());
        let stage_q = shell_single_quote(&stage.to_string_lossy());
        let ssh_user_q = shell_single_quote(&self.config.username);
        let stage_cmd = format!("install -m 0600 -o {ssh_user_q} -- {src_q} {stage_q}");
        let stage_res = self.execute_shell(&stage_cmd).await?;
        if stage_res.exit_code != 0 {
            let _ = self.execute_shell(&format!("rm -f {stage_q}")).await;
            return Err(ConnectorError::IOError(format!(
                "privileged su stage of {} failed (rc={}): {}",
                src.display(),
                stage_res.exit_code,
                String::from_utf8_lossy(&stage_res.stderr)
            )));
        }
        let dl_res = self.fetch_via_sftp(&stage, tmp_dest).await;
        self.sftp_unlink_quiet(&stage).await;
        let _ = self.execute_shell(&format!("rm -f {stage_q}")).await;
        dl_res
    }

    /// Downloads remote file `src` into local path `dest`.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if reading the remote file or writing `dest` fails.
    pub async fn fetch_file(&self, src: &Path, dest: &Path) -> Result<(), ConnectorError> {
        if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        }
        let (pid, rand_id) = unique_tmp_id();
        let tmp_dest = PathBuf::from(format!("{}.rtkk.fetch.{pid}.{rand_id}", dest.display()));
        let transfer_res = if let Some((method, target_user, pw_opt)) = self.active_escalation() {
            if method == BecomeMethod::Su {
                self.fetch_via_su_staged_sftp(src, &tmp_dest).await
            } else {
                self.fetch_via_privileged_cat(src, &tmp_dest, target_user, pw_opt)
                    .await
            }
        } else {
            self.fetch_via_sftp(src, &tmp_dest).await
        };
        if let Err(e) = transfer_res {
            let _ = tokio::fs::remove_file(&tmp_dest).await;
            return Err(e);
        }
        if let Err(e) = tokio::fs::rename(&tmp_dest, dest).await {
            let _ = tokio::fs::remove_file(&tmp_dest).await;
            return Err(ConnectorError::IOError(e.to_string()));
        }
        Ok(())
    }

    /// Streams local file `local_path` to remote `dest` with permissions `mode`.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if opening `local_path`, uploading over `SFTP`, or privileged
    /// installation fails.
    pub async fn put_file_from_local(
        &self,
        local_path: &Path,
        dest: &Path,
        mode: u32,
    ) -> Result<(), ConnectorError> {
        if self.active_escalation().is_some() {
            let (pid, rand_id) = unique_tmp_id();
            let stage = PathBuf::from(format!("/tmp/.rtkk_stage_{pid}_{rand_id:016x}"));
            if let Err(e) = self.sftp_put_local(local_path, &stage, 0o600).await {
                self.sftp_unlink_quiet(&stage).await;
                return Err(e);
            }
            return self.install_staged_remote(&stage, dest, mode).await;
        }
        self.sftp_put_local(local_path, dest, mode).await
    }

    /// Disconnects the underlying `SSH` session if initialized.
    ///
    /// # Errors
    ///
    /// Never returns an error under normal operation.
    pub async fn close(&self) -> Result<(), ConnectorError> {
        if let Some(ref session_mutex) = self.session {
            let session = session_mutex.lock().await;
            let _ = session
                .disconnect(None, "Playbook run finished", None)
                .await;
        }
        Ok(())
    }
}
