//! Top-level error types for `rtkk` operations.

/// Unified error type representing failures across `rtkk` subsystems.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RtkkError {
    /// Inventory parsing, graph construction, or host resolution failure.
    #[error("inventory: {0}")]
    Inventory(String),
    /// Playbook loading, validation, or orchestration failure.
    #[error("playbook: {0}")]
    Playbook(String),
    /// Task execution or module invocation failure.
    #[error("task: {0}")]
    Task(String),
    /// Container registry push, pull, or authentication failure.
    #[error("registry: {0}")]
    Registry(String),
    /// Host preparation or `/etc/hosts` synchronization failure.
    #[error("prep: {0}")]
    Prep(String),
    /// Logger initialization or log output failure.
    #[error("logging: {0}")]
    Logging(String),
    /// `X.509` certificate generation or verification failure.
    #[error("cert: {0}")]
    Cert(String),
    /// Workspace directory initialization or path resolution failure.
    #[error("workspace: {0}")]
    Workspace(String),
    /// Daemon `HTTP` server, authentication, or job management failure.
    #[error("daemon: {0}")]
    Daemon(String),
}
