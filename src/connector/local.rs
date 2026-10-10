use super::{CommandResult, ConnectorError};
use std::path::Path;
use tokio::process::Command;
#[derive(Default)]
pub(crate) struct LocalConnector;
impl LocalConnector {
    pub(crate) async fn init(&mut self) -> Result<(), ConnectorError> {
        async {}.await;
        Ok(())
    }
    pub(crate) async fn execute_argv_str(&self, cmd: &str) -> Result<CommandResult, ConnectorError> {
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
    pub(crate) async fn execute_argv(&self, argv: &[&str]) -> Result<CommandResult, ConnectorError> {
        let Some((prog, rest)) = argv.split_first() else {
            return Err(ConnectorError::CommandExecutionFailed(
                "empty argv".to_string(),
            ));
        };
        let child = Command::new(prog)
            .args(rest)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ConnectorError::CommandExecutionFailed(e.to_string()))?;
        let output = child
            .wait_with_output()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        Ok(CommandResult {
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
    pub(crate) async fn execute_shell(&self, script: &str) -> Result<CommandResult, ConnectorError> {
        let child = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ConnectorError::CommandExecutionFailed(e.to_string()))?;
        let output = child
            .wait_with_output()
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        Ok(CommandResult {
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
    pub(crate) async fn put_file(
        &self,
        src: &[u8],
        dest: &Path,
        mode: u32,
    ) -> Result<(), ConnectorError> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        }
        tokio::fs::write(dest, src)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let perms = std::fs::Permissions::from_mode(mode);
            tokio::fs::set_permissions(dest, perms)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        }
        Ok(())
    }
    pub(crate) async fn fetch_file(&self, src: &Path, dest: &Path) -> Result<(), ConnectorError> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        }
        tokio::fs::copy(src, dest)
            .await
            .map(|_| ())
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        Ok(())
    }
    pub(crate) async fn put_file_from_local(
        &self,
        local_path: &Path,
        dest: &Path,
        mode: u32,
    ) -> Result<(), ConnectorError> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        }
        tokio::fs::copy(local_path, dest)
            .await
            .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let perms = std::fs::Permissions::from_mode(mode);
            tokio::fs::set_permissions(dest, perms)
                .await
                .map_err(|e| ConnectorError::IOError(e.to_string()))?;
        }
        Ok(())
    }
    pub(crate) async fn close(&self) -> Result<(), ConnectorError> {
        async {}.await;
        Ok(())
    }
}
