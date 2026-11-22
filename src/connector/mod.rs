//! Target host connectors (`Local` and `SSH`), host-key policies, and privilege escalation.

use std::path::Path;

mod local;
mod ssh;

pub use local::LocalConnector;
pub use ssh::SshConnector;

/// Marker prompt passed to `sudo -p` so password prompts can be stripped from `stderr`.
pub(crate) const SUDO_PROMPT_MARKER: &str = "[RTKK_SUDO_PROMPT]";

/// Output and exit status captured from a remote or local command execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    /// Captured standard output bytes.
    pub stdout: Vec<u8>,
    /// Captured standard error bytes.
    pub stderr: Vec<u8>,
    /// Process exit status code (`-1` if terminated without an exit code).
    pub exit_code: i32,
}

/// Errors returned during connector initialization, command execution, or file transfer.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ConnectorError {
    /// Transport connection or handshake failed.
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    /// Command invocation or status retrieval failed.
    #[error("Command execution failed: {0}")]
    CommandExecutionFailed(String),
    /// Local, `SFTP`, or stream I/O failed.
    #[error("I/O error: {0}")]
    IOError(String),
    /// `SSH` credential or host-key authentication failed.
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
}

impl ConnectorError {
    /// Returns `true` if the error indicates an underlying transport/session failure.
    #[must_use]
    pub fn is_transport_error(&self) -> bool {
        match self {
            Self::ConnectionFailed(_) => true,
            Self::AuthenticationFailed(_) => false,
            Self::CommandExecutionFailed(s) | Self::IOError(s) => {
                let lower = s.to_ascii_lowercase();
                lower.contains("not initialized")
                    || lower.contains("broken pipe")
                    || lower.contains("connection reset")
                    || lower.contains("connection aborted")
                    || lower.contains("connection refused")
                    || lower.contains("timed out")
                    || lower.contains("timeout")
                    || lower.contains("disconnected")
                    || lower.contains("ssh session not initialized")
                    || (lower.contains("session") && lower.contains("closed"))
                    || (lower.contains("eof")
                        && (lower.contains("connection")
                            || lower.contains("session")
                            || lower.contains("channel")))
            }
        }
    }
}

/// Policy for verifying `SSH` server host keys against `OpenSSH` `known_hosts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostKeyPolicy {
    /// Skip server host key verification.
    #[default]
    Off,
    /// Accept and record unknown host keys, rejecting mismatched keys.
    AcceptNew,
    /// Require the server host key to already exist and match in `known_hosts`.
    Strict,
}

/// Mechanism used to escalate from an unprivileged login user to `become_user`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BecomeMethod {
    /// Escalate via `sudo` (`sudo -S` with user password or `sudo -n` passwordless).
    #[default]
    Sudo,
    /// Escalate via `su` over a `PTY` using the target account's (`root`) password.
    Su,
}

impl BecomeMethod {
    /// Parses a privilege escalation method name (`"sudo"` or `"su"`, case-insensitive).
    ///
    /// # Errors
    ///
    /// Returns an error message if `raw` is neither `"sudo"` nor `"su"`.
    pub fn parse_str(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "sudo" => Ok(Self::Sudo),
            "su" => Ok(Self::Su),
            other => Err(format!(
                "unsupported become_method '{other}'; expected 'sudo' or 'su'"
            )),
        }
    }
}

/// Privilege escalation (`sudo` / `su` / `become`) configuration for a target host.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrivilegeConfig {
    /// Whether privilege escalation is enabled.
    #[serde(default)]
    pub become_enabled: bool,
    /// Mechanism used for privilege escalation (`sudo` by default, or `su`).
    #[serde(default)]
    pub become_method: BecomeMethod,
    /// Target account to escalate into (defaults to `"root"` when `None`).
    #[serde(default)]
    pub become_user: Option<String>,
    /// Optional password sent to `sudo` (login user password) or `su` (target user password).
    #[serde(default)]
    pub become_password: Option<String>,
}

/// Connection and privilege configuration for a target host.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TargetConfig {
    /// Hostname, `IPv4`, or `IPv6` address of the target node.
    #[serde(default)]
    pub host: String,
    /// Optional explicit connector transport selection.
    #[serde(default)]
    pub connector_type: Option<ConnectorType>,
    /// Optional `SSH` configuration for remote targets.
    #[serde(default)]
    pub ssh: Option<SshConfig>,
    /// Optional privilege escalation configuration.
    #[serde(default)]
    pub privilege: Option<PrivilegeConfig>,
}

/// Transport backend used to reach a target node.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectorType {
    /// Remote execution and file transfer over `SSH` and `SFTP`.
    Ssh,
    /// Direct process execution and filesystem access on the local machine.
    Local,
}

/// `SSH` transport, credential, and host-key settings.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SshConfig {
    /// Login username for the `SSH` connection.
    pub username: String,
    /// Remote `SSH` `TCP` port (defaults to `22` when `None`).
    pub port: Option<u16>,
    /// Inline `PEM`-encoded private key content.
    pub private_key_content: Option<String>,
    /// Filesystem path to a private key file.
    pub private_key: Option<String>,
    /// Optional password for `SSH` password authentication.
    pub password: Option<String>,
    /// Whether to try password authentication when an explicit private key fails.
    #[serde(default)]
    pub private_key_fallback_password: bool,
    /// Policy for verifying the remote server's `SSH` host key.
    #[serde(default)]
    pub host_key_policy: HostKeyPolicy,
    /// Optional path to a custom `known_hosts` file (defaults to `~/.ssh/known_hosts`).
    #[serde(default)]
    pub known_hosts_file: Option<String>,
}

/// Unified connector dispatching operations to either [`LocalConnector`] or [`SshConnector`].
pub enum Connector {
    /// Local host execution backend.
    Local(LocalConnector),
    /// Remote `SSH` execution backend.
    Ssh(Box<SshConnector>),
}

impl Connector {
    /// Returns the active privilege escalation configuration, if any.
    #[must_use]
    pub fn privilege(&self) -> Option<&PrivilegeConfig> {
        match self {
            Self::Local(c) => c.privilege.as_ref(),
            Self::Ssh(c) => c.privilege.as_ref(),
        }
    }

    /// Updates the active privilege escalation configuration on this connector.
    pub fn set_privilege(&mut self, privilege: Option<PrivilegeConfig>) {
        match self {
            Self::Local(c) => c.privilege = privilege,
            Self::Ssh(c) => c.privilege = privilege,
        }
    }

    /// Initializes the underlying transport connection.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if `TCP` connection, `SSH` handshake, host-key verification,
    /// or authentication fails.
    pub async fn init(&mut self) -> Result<(), ConnectorError> {
        match self {
            Self::Local(c) => c.init().await,
            Self::Ssh(c) => c.init().await,
        }
    }

    /// Executes a command string on the target host.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if command parsing, spawning, or stream I/O fails.
    pub async fn execute_command(&self, cmd: &str) -> Result<CommandResult, ConnectorError> {
        match self {
            Self::Local(c) => c.execute_argv_str(cmd).await,
            Self::Ssh(c) => c.execute_command(cmd).await,
        }
    }

    /// Executes an argument vector on the target host without shell word-splitting on local hosts.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if `argv` is empty or execution fails.
    pub async fn execute_argv(&self, argv: &[&str]) -> Result<CommandResult, ConnectorError> {
        match self {
            Self::Local(c) => c.execute_argv(argv).await,
            Self::Ssh(c) => c.execute_argv(argv).await,
        }
    }

    /// Executes a `/bin/sh -c` script on the target host.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if process execution or stream I/O fails.
    pub async fn execute_shell(&self, script: &str) -> Result<CommandResult, ConnectorError> {
        match self {
            Self::Local(c) => c.execute_shell(script).await,
            Self::Ssh(c) => c.execute_shell(script).await,
        }
    }

    /// Writes `src` bytes to `dest` on the target host with `POSIX` permissions `mode`.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if writing or setting permissions fails.
    pub async fn put_file(&self, src: &[u8], dest: &Path, mode: u32) -> Result<(), ConnectorError> {
        match self {
            Self::Local(c) => c.put_file(src, dest, mode).await,
            Self::Ssh(c) => c.put_file(src, dest, mode).await,
        }
    }

    /// Fetches remote file `src` from the target host into local path `dest`.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if reading the remote file or writing the local file fails.
    pub async fn fetch_file(&self, src: &Path, dest: &Path) -> Result<(), ConnectorError> {
        match self {
            Self::Local(c) => c.fetch_file(src, dest).await,
            Self::Ssh(c) => c.fetch_file(src, dest).await,
        }
    }

    /// Streams local file `local_path` to `dest` on the target host with permissions `mode`.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if reading `local_path`, transferring data, or setting mode fails.
    pub async fn put_file_from_local(
        &self,
        local_path: &Path,
        dest: &Path,
        mode: u32,
    ) -> Result<(), ConnectorError> {
        match self {
            Self::Local(c) => c.put_file_from_local(local_path, dest, mode).await,
            Self::Ssh(c) => c.put_file_from_local(local_path, dest, mode).await,
        }
    }

    /// Closes the underlying session if open.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError`] if closing transport resources fails.
    pub async fn close(&self) -> Result<(), ConnectorError> {
        match self {
            Self::Local(c) => c.close().await,
            Self::Ssh(c) => c.close().await,
        }
    }
}

/// Formats `host` and `port` as a socket address string, bracketing bare `IPv6` literals.
pub(crate) fn format_host_port(host: &str, port: u16) -> String {
    let trimmed = host.trim();
    if trimmed.contains(':') && !(trimmed.starts_with('[') && trimmed.ends_with(']')) {
        format!("[{trimmed}]:{port}")
    } else {
        format!("{trimmed}:{port}")
    }
}

/// Returns the maximum bytes allowed for captured `stdout` or `stderr` (`RTKK_MAX_CMD_OUTPUT_BYTES`, default 64 MiB).
pub(crate) fn max_cmd_output_bytes() -> usize {
    const DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;
    match std::env::var("RTKK_MAX_CMD_OUTPUT_BYTES") {
        Ok(v) => v
            .trim()
            .parse::<usize>()
            .unwrap_or(DEFAULT_MAX_BYTES)
            .max(1024),
        Err(_) => DEFAULT_MAX_BYTES,
    }
}

/// Generates a collision-resistant `(pid, rand_u64)` pair for temporary staging filenames.
pub(crate) fn unique_tmp_id() -> (u32, u64) {
    use std::hash::{BuildHasher as _, Hasher as _};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let pid = std::process::id();
    let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(pid);
    hasher.write_u64(seq);
    if let Ok(dur) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        hasher.write_u128(dur.as_nanos());
    }
    (pid, hasher.finish())
}

/// Strips [`SUDO_PROMPT_MARKER`] (and an optional trailing newline) from captured `stderr` bytes.
pub(crate) fn strip_sudo_prompt(stderr: Vec<u8>) -> Vec<u8> {
    if stderr.is_empty() {
        return stderr;
    }
    let marker = SUDO_PROMPT_MARKER.as_bytes();
    let mut out = Vec::with_capacity(stderr.len());
    let mut rest = stderr.as_slice();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix(marker) {
            rest = after
                .strip_prefix(b"\r\n")
                .or_else(|| after.strip_prefix(b"\n"))
                .unwrap_or(after);
        } else if let Some((&byte, tail)) = rest.split_first() {
            out.push(byte);
            rest = tail;
        } else {
            break;
        }
    }
    out
}

/// Wraps a shell command with `sudo` for `target_user`.
pub(crate) fn build_sudo_shell_cmd(
    target_user: &str,
    inner_cmd: &str,
    has_password: bool,
) -> String {
    let user_q = crate::utils::shell_single_quote(target_user);
    let cmd_q = crate::utils::shell_single_quote(inner_cmd);
    if has_password {
        let prompt_q = crate::utils::shell_single_quote(SUDO_PROMPT_MARKER);
        format!("sudo -S -H -p {prompt_q} -u {user_q} -- /bin/sh -c {cmd_q}")
    } else {
        format!("sudo -n -H -u {user_q} -- /bin/sh -c {cmd_q}")
    }
}

/// Framed command specification for `su` privilege escalation over a `PTY`.
pub(crate) struct SuFrameSpec {
    /// Complete outer shell command invoking `LC_ALL=C su`.
    pub cmd: String,
    /// Unique marker printed right before `inner_cmd` `stdout` begins.
    pub start_marker: String,
    /// Unique marker printed between `inner_cmd` `stdout` and captured `stderr`.
    pub err_marker: String,
}

/// Builds a `PTY`-safe `su` command that normalizes `HOME`/`PATH`, detaches inner `stdin`,
/// and frames `stdout` and `stderr` with unique boundary markers.
pub(crate) fn build_su_shell_cmd(target_user: &str, inner_cmd: &str) -> SuFrameSpec {
    let (pid, rand_id) = unique_tmp_id();
    let start_marker = format!("__RTKK_SU_START_{pid}_{rand_id:016x}__");
    let err_marker = format!("__RTKK_SU_ERR_{pid}_{rand_id:016x}__");
    let err_file = format!("/tmp/.rtkk_su_err_{pid}_{rand_id:016x}");

    let user_q = crate::utils::shell_single_quote(target_user);
    let inner_q = crate::utils::shell_single_quote(inner_cmd);
    let err_q = crate::utils::shell_single_quote(&err_file);

    let payload = format!(
        "export PATH='/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin'; \
         export USER={user_q} LOGNAME={user_q}; \
         __rtkk_h=\"$(getent passwd {user_q} 2>/dev/null | cut -d: -f6)\"; \
         if [ -n \"$__rtkk_h\" ]; then export HOME=\"$__rtkk_h\"; \
         elif [ {user_q} = 'root' ]; then export HOME='/root'; fi; \
         unset __rtkk_h; \
         __rtkk_u=\"$(umask)\"; umask 0077; : > {err_q}; umask \"$__rtkk_u\"; unset __rtkk_u; \
         printf '%s\\n' '{start_marker}'; \
         ( /bin/sh -c {inner_q} ) </dev/null 2>{err_q}; \
         __rtkk_rc=$?; \
         printf '\\n%s\\n' '{err_marker}'; \
         cat {err_q} 2>/dev/null || true; \
         rm -f {err_q} 2>/dev/null || true; \
         exit \"$__rtkk_rc\""
    );
    let payload_q = crate::utils::shell_single_quote(&payload);
    let cmd = format!("LC_ALL=C su -s /bin/sh -c {payload_q} {user_q}");
    SuFrameSpec {
        cmd,
        start_marker,
        err_marker,
    }
}

/// Returns `true` if `haystack` contains `needle`.
pub(crate) fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    find_subslice(haystack, needle).is_some()
}

/// Returns `true` if `buf` contains an `ASCII` `"Password:"` prompt from `su`.
pub(crate) fn contains_password_prompt(buf: &[u8]) -> bool {
    buf.windows(9).any(|w| w.eq_ignore_ascii_case(b"password:"))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn rfind_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(haystack.len());
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

fn strip_leading_line_break(slice: &[u8]) -> &[u8] {
    slice
        .strip_prefix(b"\r\n")
        .or_else(|| slice.strip_prefix(b"\n"))
        .unwrap_or(slice)
}

fn strip_trailing_single_line_break(slice: &[u8]) -> &[u8] {
    slice
        .strip_suffix(b"\r\n")
        .or_else(|| slice.strip_suffix(b"\n"))
        .unwrap_or(slice)
}

/// Parses raw `PTY` output from [`build_su_shell_cmd`] into separated `stdout` and `stderr`,
/// or returns [`ConnectorError::AuthenticationFailed`] if `su` rejected credentials before
/// reaching `start_marker`.
pub(crate) fn parse_su_framed_output(
    raw: &[u8],
    start_marker: &str,
    err_marker: &str,
    exit_code: i32,
) -> Result<CommandResult, ConnectorError> {
    let start_bytes = start_marker.as_bytes();
    let Some(start_idx) = find_subslice(raw, start_bytes) else {
        let cleaned: Vec<u8> = raw.iter().copied().filter(|&b| b != b'\r').collect();
        let text = String::from_utf8_lossy(&cleaned);
        let detail = text
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.is_empty()
                    && !line.eq_ignore_ascii_case("password:")
                    && !line
                        .strip_prefix("Password:")
                        .or_else(|| line.strip_prefix("password:"))
                        .is_some_and(|rest| rest.trim().is_empty())
            })
            .map(|line| {
                line.strip_prefix("Password:")
                    .or_else(|| line.strip_prefix("password:"))
                    .map_or(line, str::trim)
            })
            .collect::<Vec<_>>()
            .join("; ");
        let msg = if detail.is_empty() {
            format!("su privilege escalation failed before command start (rc={exit_code})")
        } else {
            format!("su privilege escalation failed (rc={exit_code}): {detail}")
        };
        return Err(ConnectorError::AuthenticationFailed(msg));
    };

    let after_marker_pos = start_idx.saturating_add(start_bytes.len());
    let after_start = strip_leading_line_break(raw.get(after_marker_pos..).unwrap_or(&[]));

    let err_bytes = err_marker.as_bytes();
    if let Some(err_idx) = rfind_subslice(after_start, err_bytes) {
        let before_err = after_start.get(..err_idx).unwrap_or(&[]);
        let stdout = strip_trailing_single_line_break(before_err).to_vec();
        let after_err_pos = err_idx.saturating_add(err_bytes.len());
        let stderr =
            strip_leading_line_break(after_start.get(after_err_pos..).unwrap_or(&[])).to_vec();
        Ok(CommandResult {
            stdout,
            stderr,
            exit_code,
        })
    } else {
        Ok(CommandResult {
            stdout: after_start.to_vec(),
            stderr: Vec::new(),
            exit_code,
        })
    }
}

/// Reads all bytes from `reader` up to `max_bytes`, failing if the cap is exceeded.
pub(crate) async fn read_stream_bounded<R>(
    reader: &mut R,
    max_bytes: usize,
    stream_name: &str,
) -> Result<Vec<u8>, ConnectorError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt as _;
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 8192];
    loop {
        let n = reader
            .read(&mut chunk)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        if n == 0 {
            break;
        }
        if buf.len().saturating_add(n) > max_bytes {
            return Err(ConnectorError::IOError(format!(
                "command {stream_name} exceeded RTKK_MAX_CMD_OUTPUT_BYTES ({max_bytes})"
            )));
        }
        if let Some(slice) = chunk.get(..n) {
            buf.extend_from_slice(slice);
        }
    }
    Ok(buf)
}

/// Returns `true` if `host` resolves to loopback or an `IP` address assigned to a local interface.
pub async fn is_local_ip(host: &str) -> bool {
    let bare = host
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or_else(|| host.trim());
    if bare == "localhost" || bare == "127.0.0.1" || bare == "::1" {
        return true;
    }
    let resolved_ips = if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        vec![ip]
    } else {
        let addr_str = format_host_port(bare, 0);
        if let Ok(addrs) = tokio::net::lookup_host(&addr_str).await {
            addrs.map(|socket_addr| socket_addr.ip()).collect()
        } else {
            Vec::new()
        }
    };
    if resolved_ips.is_empty() {
        return false;
    }
    if let Ok(local_addrs) = get_if_addrs::get_if_addrs() {
        for resolved_ip in resolved_ips {
            if resolved_ip.is_loopback() {
                return true;
            }
            for local_addr in &local_addrs {
                if local_addr.ip() == resolved_ip {
                    return true;
                }
            }
        }
    }
    false
}

/// Creates and initializes a [`Connector`] for the provided [`TargetConfig`].
///
/// # Errors
///
/// Returns [`ConnectorError`] if `SSH` settings are missing for a remote host or if transport
/// initialization fails.
pub async fn create_connector(config: &TargetConfig) -> Result<Connector, ConnectorError> {
    if let Some(ref conn_type) = config.connector_type {
        match conn_type {
            ConnectorType::Local => {
                let mut conn = Connector::Local(LocalConnector {
                    privilege: config.privilege.clone(),
                });
                conn.init().await?;
                return Ok(conn);
            }
            ConnectorType::Ssh => {
                let ssh_config = config.ssh.clone().ok_or_else(|| {
                    ConnectorError::ConnectionFailed(
                        "SSH configuration missing for SSH target type".to_string(),
                    )
                })?;
                let mut conn = Connector::Ssh(Box::new(SshConnector {
                    config: ssh_config,
                    host: config.host.clone(),
                    privilege: config.privilege.clone(),
                    session: None,
                }));
                conn.init().await?;
                return Ok(conn);
            }
        }
    }
    if is_local_ip(&config.host).await {
        let mut conn = Connector::Local(LocalConnector {
            privilege: config.privilege.clone(),
        });
        conn.init().await?;
        return Ok(conn);
    }
    let ssh_config = config.ssh.clone().ok_or_else(|| {
        ConnectorError::ConnectionFailed("SSH configuration missing for remote target".to_string())
    })?;
    let mut conn = Connector::Ssh(Box::new(SshConnector {
        config: ssh_config,
        host: config.host.clone(),
        privilege: config.privilege.clone(),
        session: None,
    }));
    conn.init().await?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::{
        BecomeMethod, ConnectorError, build_su_shell_cmd, contains_password_prompt,
        parse_su_framed_output,
    };

    #[test]
    fn become_method_parses_case_insensitively_and_rejects_unknown() -> Result<(), String> {
        if BecomeMethod::parse_str("sudo")? != BecomeMethod::Sudo {
            return Err("expected Sudo".to_string());
        }
        if BecomeMethod::parse_str("  SU ")? != BecomeMethod::Su {
            return Err("expected Su".to_string());
        }
        if BecomeMethod::parse_str("doas").is_ok() {
            return Err("expected error for unsupported become_method 'doas'".to_string());
        }
        Ok(())
    }

    #[test]
    fn su_frame_parser_preserves_exact_stdout_stderr_and_binary_bytes() -> Result<(), String> {
        let spec = build_su_shell_cmd("root", "printf 'true'");
        if !spec.cmd.contains("LC_ALL=C su -s /bin/sh -c") {
            return Err("expected POSIX su invocation".to_string());
        }

        let raw_no_nl = format!(
            "Password: \n{}\ntrue\n{}\n[WARNING]: sample warning\n",
            spec.start_marker, spec.err_marker
        );
        let res_no_nl = parse_su_framed_output(
            raw_no_nl.as_bytes(),
            &spec.start_marker,
            &spec.err_marker,
            0,
        )
        .map_err(|e| e.to_string())?;
        if res_no_nl.stdout != b"true" {
            return Err(format!(
                "expected exact b\"true\", got {:?}",
                res_no_nl.stdout
            ));
        }
        if res_no_nl.stderr != b"[WARNING]: sample warning\n" {
            return Err(format!("unexpected stderr: {:?}", res_no_nl.stderr));
        }

        let mut bin_raw = Vec::new();
        bin_raw.extend_from_slice(b"Password: \r\n");
        bin_raw.extend_from_slice(spec.start_marker.as_bytes());
        bin_raw.extend_from_slice(b"\r\n\x00\x01\xfe\xff\n");
        bin_raw.extend_from_slice(spec.err_marker.as_bytes());
        bin_raw.extend_from_slice(b"\r\n");
        let res_bin = parse_su_framed_output(&bin_raw, &spec.start_marker, &spec.err_marker, 0)
            .map_err(|e| e.to_string())?;
        if res_bin.stdout != b"\x00\x01\xfe\xff" || !res_bin.stderr.is_empty() {
            return Err(format!("binary stdout corrupted: {:?}", res_bin.stdout));
        }
        Ok(())
    }

    #[test]
    fn su_frame_parser_reports_authentication_failure_when_marker_missing() -> Result<(), String> {
        if !contains_password_prompt(b"Password: ") {
            return Err("expected password prompt detection".to_string());
        }
        let raw_fail = b"Password: \nsu: Authentication failure\n";
        let err = parse_su_framed_output(raw_fail, "__START__", "__ERR__", 1);
        match err {
            Err(ConnectorError::AuthenticationFailed(msg))
                if msg.contains("Authentication failure") =>
            {
                Ok(())
            }
            other => Err(format!("expected AuthenticationFailed, got {other:?}")),
        }
    }
}
