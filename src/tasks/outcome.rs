use crate::connector::ConnectorError;
#[derive(Debug)]
pub(crate) enum TaskModuleError {
    Transport(ConnectorError),
    Message(String),
}
impl TaskModuleError {
    pub(crate) fn from_connector(e: ConnectorError) -> Self {
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
