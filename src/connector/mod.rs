use std::path::Path;
mod local;
mod ssh;
pub(crate) use local::LocalConnector;
pub(crate) use ssh::SshConnector;
#[derive(Debug, Clone)]
pub(crate) struct CommandResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
}
#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum ConnectorError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Command execution failed: {0}")]
    CommandExecutionFailed(String),
    #[error("I/O error: {0}")]
    IOError(String),
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
}
impl ConnectorError {
    #[must_use]
    pub(crate) fn is_transport_error(&self) -> bool {
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct TargetConfig {
    pub host: String,
    pub connector_type: Option<ConnectorType>,
    pub ssh: Option<SshConfig>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ConnectorType {
    Ssh,
    Local,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SshConfig {
    pub username: String,
    pub port: Option<u16>,
    pub private_key_content: Option<String>,
    pub private_key: Option<String>,
    pub password: Option<String>,
    #[serde(default)]
    pub private_key_fallback_password: bool,
}
pub(crate) enum Connector {
    Local(LocalConnector),
    Ssh(SshConnector),
}
impl Connector {
    pub(crate) async fn init(&mut self) -> Result<(), ConnectorError> {
        match self {
            Self::Local(c) => c.init().await,
            Self::Ssh(c) => c.init().await,
        }
    }
    pub(crate) async fn execute_command(&self, cmd: &str) -> Result<CommandResult, ConnectorError> {
        match self {
            Self::Local(c) => c.execute_argv_str(cmd).await,
            Self::Ssh(c) => c.execute_command(cmd).await,
        }
    }
    pub(crate) async fn execute_argv(&self, argv: &[&str]) -> Result<CommandResult, ConnectorError> {
        match self {
            Self::Local(c) => c.execute_argv(argv).await,
            Self::Ssh(c) => c.execute_argv(argv).await,
        }
    }
    pub(crate) async fn execute_shell(&self, script: &str) -> Result<CommandResult, ConnectorError> {
        match self {
            Self::Local(c) => c.execute_shell(script).await,
            Self::Ssh(c) => c.execute_shell(script).await,
        }
    }
    pub(crate) async fn put_file(
        &self,
        src: &[u8],
        dest: &Path,
        mode: u32,
    ) -> Result<(), ConnectorError> {
        match self {
            Self::Local(c) => c.put_file(src, dest, mode).await,
            Self::Ssh(c) => c.put_file(src, dest, mode).await,
        }
    }
    pub(crate) async fn fetch_file(&self, src: &Path, dest: &Path) -> Result<(), ConnectorError> {
        match self {
            Self::Local(c) => c.fetch_file(src, dest).await,
            Self::Ssh(c) => c.fetch_file(src, dest).await,
        }
    }
    pub(crate) async fn put_file_from_local(
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
    pub(crate) async fn close(&self) -> Result<(), ConnectorError> {
        match self {
            Self::Local(c) => c.close().await,
            Self::Ssh(c) => c.close().await,
        }
    }
}
pub(crate) async fn is_local_ip(host: &str) -> bool {
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        return true;
    }
    let resolved_ips = if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        vec![ip]
    } else {
        let addr_str = format!("{host}:0");
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
pub(crate) async fn create_connector(config: &TargetConfig) -> Result<Connector, ConnectorError> {
    if let Some(ref conn_type) = config.connector_type {
        match conn_type {
            ConnectorType::Local => {
                let mut conn = Connector::Local(LocalConnector);
                conn.init().await?;
                return Ok(conn);
            }
            ConnectorType::Ssh => {
                let ssh_config = config.ssh.clone().ok_or_else(|| {
                    ConnectorError::ConnectionFailed(
                        "SSH configuration missing for SSH target type".to_string(),
                    )
                })?;
                let mut conn = Connector::Ssh(SshConnector {
                    config: ssh_config,
                    host: config.host.clone(),
                    session: None,
                });
                conn.init().await?;
                return Ok(conn);
            }
        }
    }
    if is_local_ip(&config.host).await {
        let mut conn = Connector::Local(LocalConnector);
        conn.init().await?;
        return Ok(conn);
    }
    let ssh_config = config.ssh.clone().ok_or_else(|| {
        ConnectorError::ConnectionFailed("SSH configuration missing for remote target".to_string())
    })?;
    let mut conn = Connector::Ssh(SshConnector {
        config: ssh_config,
        host: config.host.clone(),
        session: None,
    });
    conn.init().await?;
    Ok(conn)
}
