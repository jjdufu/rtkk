use super::{
    BecomeMethod, CommandResult, ConnectorError, PrivilegeConfig, SUDO_PROMPT_MARKER,
    build_su_shell_cmd, contains_password_prompt, contains_subslice, max_cmd_output_bytes,
    parse_su_framed_output, read_stream_bounded, strip_sudo_prompt, unique_tmp_id,
};
use crate::utils::shell_single_quote;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Connector that executes commands and file transfers directly on the local machine.
#[derive(Debug, Clone, Default)]
pub struct LocalConnector {
    /// Optional privilege escalation (`sudo` / `become`) configuration.
    pub privilege: Option<PrivilegeConfig>,
}

fn is_local_effective_root() -> bool {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return false;
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            let mut fields = rest.split_whitespace();
            let _real = fields.next();
            if let Some(effective) = fields.next() {
                return effective == "0";
            }
        }
    }
    false
}

async fn run_local_command(
    mut cmd: Command,
    stdin_data: Option<&[u8]>,
    strip_prompt: bool,
) -> Result<CommandResult, ConnectorError> {
    use tokio::io::AsyncWriteExt as _;

    cmd.kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if stdin_data.is_some() {
        cmd.stdin(std::process::Stdio::piped());
    } else {
        cmd.stdin(std::process::Stdio::null());
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| ConnectorError::CommandExecutionFailed(e.to_string()))?;
    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| ConnectorError::IOError("failed to capture local stdout".to_string()))?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| ConnectorError::IOError("failed to capture local stderr".to_string()))?;
    let child_stdin = child.stdin.take();
    let max_bytes = max_cmd_output_bytes();

    let stdin_fut = async move {
        if let (Some(mut pipe), Some(data)) = (child_stdin, stdin_data) {
            if let Err(e) = pipe.write_all(data).await
                && e.kind() != std::io::ErrorKind::BrokenPipe
            {
                return Err(ConnectorError::IOError(e.to_string()));
            }
            let _ = pipe.shutdown().await;
        }
        Ok::<(), ConnectorError>(())
    };
    let stdout_fut = read_stream_bounded(&mut stdout_pipe, max_bytes, "stdout");
    let stderr_fut = read_stream_bounded(&mut stderr_pipe, max_bytes, "stderr");

    let ((), stdout, raw_stderr) = tokio::try_join!(stdin_fut, stdout_fut, stderr_fut)?;
    let status = child
        .wait()
        .await
        .map_err(|e| ConnectorError::IOError(e.to_string()))?;
    let stderr = if strip_prompt {
        strip_sudo_prompt(raw_stderr)
    } else {
        raw_stderr
    };
    Ok(CommandResult {
        stdout,
        stderr,
        exit_code: status.code().unwrap_or(-1),
    })
}

fn set_mode_0600_and_build_stage() -> PathBuf {
    let (pid, rand_id) = unique_tmp_id();
    PathBuf::from(format!("/tmp/.rtkk_stage_{pid}_{rand_id:016x}"))
}

#[cfg(unix)]
async fn set_unix_mode(path: &Path, mode: u32) -> Result<(), ConnectorError> {
    use std::os::unix::fs::PermissionsExt as _;
    let perms = std::fs::Permissions::from_mode(mode);
    tokio::fs::set_permissions(path, perms)
        .await
        .map_err(|e| ConnectorError::IOError(e.to_string()))
}

#[cfg(not(unix))]
async fn set_unix_mode(_path: &Path, _mode: u32) -> Result<(), ConnectorError> {
    async {}.await;
    Ok(())
}

impl LocalConnector {
    /// Creates a new [`LocalConnector`] without privilege escalation.
    #[must_use]
    pub const fn new() -> Self {
        Self { privilege: None }
    }

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
        if target_user == "root" && is_local_effective_root() {
            return None;
        }
        if std::env::var("USER").as_deref() == Ok(target_user) {
            return None;
        }
        Some((
            priv_cfg.become_method,
            target_user,
            priv_cfg.become_password.as_deref(),
        ))
    }

    async fn execute_local_su(
        target_user: &str,
        inner_cmd: &str,
        pw_opt: Option<&str>,
    ) -> Result<CommandResult, ConnectorError> {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let pw = pw_opt.filter(|s| !s.is_empty()).ok_or_else(|| {
            ConnectorError::AuthenticationFailed(
                "become_method 'su' requires non-empty ansible_become_pass (target user password)"
                    .to_string(),
            )
        })?;
        let frame = build_su_shell_cmd(target_user, inner_cmd);
        let pty_wrapped = format!(
            "stty -echo -onlcr -ocrnl 2>/dev/null || true; {}",
            frame.cmd
        );

        let mut cmd = Command::new("script");
        if cfg!(target_os = "macos") {
            cmd.args(["-q", "/dev/null", "/bin/sh", "-c", &pty_wrapped]);
        } else {
            cmd.args(["-qefc", &pty_wrapped, "/dev/null"]);
        }
        cmd.kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            ConnectorError::CommandExecutionFailed(format!("failed to spawn script for su: {e}"))
        })?;
        let mut stdout_pipe = child.stdout.take().ok_or_else(|| {
            ConnectorError::IOError("failed to capture local su stdout".to_string())
        })?;
        let mut stderr_pipe = child.stderr.take().ok_or_else(|| {
            ConnectorError::IOError("failed to capture local su stderr".to_string())
        })?;
        let mut stdin_pipe = child.stdin.take().ok_or_else(|| {
            ConnectorError::IOError("failed to capture local su stdin".to_string())
        })?;

        let max_bytes = max_cmd_output_bytes();
        let pw_line = format!("{pw}\n");
        let start_bytes = frame.start_marker.as_bytes().to_vec();

        let read_write_fut = async move {
            let mut raw_buf = Vec::new();
            let mut chunk = vec![0u8; 8192];
            let mut password_sent = false;
            loop {
                let n = stdout_pipe
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
                    if contains_subslice(&raw_buf, &start_bytes) {
                        password_sent = true;
                    } else if contains_password_prompt(&raw_buf) {
                        if let Err(e) = stdin_pipe.write_all(pw_line.as_bytes()).await
                            && e.kind() != std::io::ErrorKind::BrokenPipe
                        {
                            return Err(ConnectorError::IOError(e.to_string()));
                        }
                        let _ = stdin_pipe.flush().await;
                        password_sent = true;
                    }
                }
            }
            drop(stdin_pipe);
            Ok::<Vec<u8>, ConnectorError>(raw_buf)
        };
        let stderr_fut = read_stream_bounded(&mut stderr_pipe, max_bytes, "stderr");
        let (raw_buf, extra_stderr) = tokio::try_join!(read_write_fut, stderr_fut)?;
        let status = child
            .wait()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        let exit_code = status.code().unwrap_or(-1);
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

    /// Initializes the local connector (no-op).
    ///
    /// # Errors
    ///
    /// Never returns an error under normal operation.
    pub async fn init(&mut self) -> Result<(), ConnectorError> {
        async {}.await;
        Ok(())
    }

    /// Splits `cmd` into `POSIX` words and executes it via [`Self::execute_argv`].
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if word-splitting fails, `cmd` is empty, or execution fails.
    pub async fn execute_argv_str(&self, cmd: &str) -> Result<CommandResult, ConnectorError> {
        let argv = shell_words::split(cmd).map_err(|e| {
            ConnectorError::CommandExecutionFailed(format!("failed to parse command argv: {e}"))
        })?;
        if argv.is_empty() {
            return Err(ConnectorError::CommandExecutionFailed(
                "empty command".to_string(),
            ));
        }
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.execute_argv(&refs).await
    }

    /// Executes an argument slice directly without shell interpretation.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if `argv` is empty or spawning/reading output fails.
    pub async fn execute_argv(&self, argv: &[&str]) -> Result<CommandResult, ConnectorError> {
        let Some((prog, rest)) = argv.split_first() else {
            return Err(ConnectorError::CommandExecutionFailed(
                "empty argv".to_string(),
            ));
        };
        if let Some((method, target_user, pw_opt)) = self.active_escalation() {
            if method == BecomeMethod::Su {
                let joined = shell_words::join(argv);
                return Self::execute_local_su(target_user, &joined, pw_opt).await;
            }
            let mut cmd = Command::new("sudo");
            if let Some(pw) = pw_opt {
                cmd.args([
                    "-S",
                    "-H",
                    "-p",
                    SUDO_PROMPT_MARKER,
                    "-u",
                    target_user,
                    "--",
                ]);
                cmd.arg(prog).args(rest);
                let stdin_buf = format!("{pw}\n");
                return run_local_command(cmd, Some(stdin_buf.as_bytes()), true).await;
            }
            cmd.args(["-n", "-H", "-u", target_user, "--"]);
            cmd.arg(prog).args(rest);
            return run_local_command(cmd, None, false).await;
        }

        let mut cmd = Command::new(prog);
        cmd.args(rest);
        run_local_command(cmd, None, false).await
    }

    /// Executes `script` via `/bin/sh -c` on the local machine.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if spawning `/bin/sh` or reading output fails.
    pub async fn execute_shell(&self, script: &str) -> Result<CommandResult, ConnectorError> {
        if let Some((method, target_user, pw_opt)) = self.active_escalation() {
            if method == BecomeMethod::Su {
                return Self::execute_local_su(target_user, script, pw_opt).await;
            }
            let mut cmd = Command::new("sudo");
            if let Some(pw) = pw_opt {
                cmd.args([
                    "-S",
                    "-H",
                    "-p",
                    SUDO_PROMPT_MARKER,
                    "-u",
                    target_user,
                    "--",
                    "/bin/sh",
                    "-c",
                    script,
                ]);
                let stdin_buf = format!("{pw}\n");
                return run_local_command(cmd, Some(stdin_buf.as_bytes()), true).await;
            }
            cmd.args(["-n", "-H", "-u", target_user, "--", "/bin/sh", "-c", script]);
            return run_local_command(cmd, None, false).await;
        }

        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(script);
        run_local_command(cmd, None, false).await
    }

    async fn install_staged_file(
        &self,
        stage: &Path,
        dest: &Path,
        mode: u32,
    ) -> Result<(), ConnectorError> {
        let stage_str = stage.to_string_lossy();
        let dest_str = dest.to_string_lossy();
        let stage_q = shell_single_quote(&stage_str);
        let dest_q = shell_single_quote(&dest_str);
        let cmd = if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
            let parent_q = shell_single_quote(&parent.to_string_lossy());
            format!(
                "mkdir -p {parent_q} && install -m {mode:o} {stage_q} {dest_q} && rm -f {stage_q}"
            )
        } else {
            format!("install -m {mode:o} {stage_q} {dest_q} && rm -f {stage_q}")
        };
        let res = self.execute_shell(&cmd).await;
        let _ = tokio::fs::remove_file(stage).await;
        match res {
            Ok(r) if r.exit_code == 0 => Ok(()),
            Ok(r) => {
                let _ = self.execute_shell(&format!("rm -f {stage_q}")).await;
                Err(ConnectorError::IOError(format!(
                    "privileged install to {} failed (rc={}): {}",
                    dest.display(),
                    r.exit_code,
                    String::from_utf8_lossy(&r.stderr)
                )))
            }
            Err(e) => {
                let _ = self.execute_shell(&format!("rm -f {stage_q}")).await;
                Err(e)
            }
        }
    }

    /// Writes `src` bytes to `dest` with permissions `mode`.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if directory creation, file writing, or mode setting fails.
    pub async fn put_file(&self, src: &[u8], dest: &Path, mode: u32) -> Result<(), ConnectorError> {
        if self.active_escalation().is_some() {
            let stage = set_mode_0600_and_build_stage();
            tokio::fs::write(&stage, src)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
            if let Err(e) = set_unix_mode(&stage, 0o600).await {
                let _ = tokio::fs::remove_file(&stage).await;
                return Err(e);
            }
            return self.install_staged_file(&stage, dest, mode).await;
        }
        if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        }
        tokio::fs::write(dest, src)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        set_unix_mode(dest, mode).await
    }

    /// Copies file `src` into `dest` on the local filesystem.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if reading `src` or writing `dest` fails.
    pub async fn fetch_file(&self, src: &Path, dest: &Path) -> Result<(), ConnectorError> {
        if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        }
        if self.active_escalation().is_none() {
            tokio::fs::copy(src, dest)
                .await
                .map(|_| ())
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
            return Ok(());
        }
        let (pid, rand_id) = unique_tmp_id();
        let tmp_dest = PathBuf::from(format!("{}.rtkk.fetch.{pid}.{rand_id}", dest.display()));
        let src_q = shell_single_quote(&src.to_string_lossy());
        let tmp_q = shell_single_quote(&tmp_dest.to_string_lossy());
        let res = self
            .execute_shell(&format!("cat -- {src_q} > {tmp_q}"))
            .await?;
        if res.exit_code != 0 {
            let _ = tokio::fs::remove_file(&tmp_dest).await;
            return Err(ConnectorError::IOError(format!(
                "privileged fetch {} failed (rc={}): {}",
                src.display(),
                res.exit_code,
                String::from_utf8_lossy(&res.stderr)
            )));
        }
        if let Err(e) = tokio::fs::rename(&tmp_dest, dest).await {
            let _ = tokio::fs::remove_file(&tmp_dest).await;
            return Err(ConnectorError::IOError(e.to_string()));
        }
        Ok(())
    }

    /// Copies local file `local_path` to `dest` with permissions `mode`.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if copying or setting permissions fails.
    pub async fn put_file_from_local(
        &self,
        local_path: &Path,
        dest: &Path,
        mode: u32,
    ) -> Result<(), ConnectorError> {
        if self.active_escalation().is_some() {
            let stage = set_mode_0600_and_build_stage();
            tokio::fs::copy(local_path, &stage)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
            if let Err(e) = set_unix_mode(&stage, 0o600).await {
                let _ = tokio::fs::remove_file(&stage).await;
                return Err(e);
            }
            return self.install_staged_file(&stage, dest, mode).await;
        }
        if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        }
        tokio::fs::copy(local_path, dest)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        set_unix_mode(dest, mode).await
    }

    /// Closes the local connector (no-op).
    ///
    /// # Errors
    ///
    /// Never returns an error under normal operation.
    pub async fn close(&self) -> Result<(), ConnectorError> {
        async {}.await;
        Ok(())
    }
}
