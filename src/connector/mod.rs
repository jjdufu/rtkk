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

/// Privilege escalation (`sudo` / `become`) configuration for a target host.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrivilegeConfig {
    /// Whether privilege escalation is enabled.
    #[serde(default)]
    pub become_enabled: bool,
    /// Target account to escalate into (defaults to `"root"` when `None`).
    #[serde(default)]
    pub become_user: Option<String>,
    /// Optional password sent to `sudo` via standard input.
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
