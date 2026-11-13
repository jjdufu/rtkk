use crate::connector::ConnectorError;

/// Error returned when executing a playbook task module.
#[derive(Debug, Clone)]
pub enum TaskModuleError {
    /// Underlying transport/session failure eligible for connector reconnection.
    Transport(ConnectorError),
    /// Logical, validation, or remote command failure message.
    Message(String),
}

impl TaskModuleError {
    /// Classifies a [`ConnectorError`] into either [`Self::Transport`] or [`Self::Message`].
    #[must_use]
    pub fn from_connector(e: ConnectorError) -> Self {
        if e.is_transport_error() {
            Self::Transport(e)
        } else {
            Self::Message(e.to_string())
        }
    }
}

impl From<String> for TaskModuleError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

impl From<&str> for TaskModuleError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_string())
    }
}

impl std::fmt::Display for TaskModuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "{e}"),
            Self::Message(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for TaskModuleError {}
