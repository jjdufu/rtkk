#[derive(Debug, thiserror::Error)]
pub(crate) enum RtkkError {
    #[error("inventory: {0}")]
    Inventory(String),
    #[error("playbook: {0}")]
    Playbook(String),
    #[error("task: {0}")]
    Task(String),
    #[error("registry: {0}")]
    Registry(String),
    #[error("prep: {0}")]
    Prep(String),
    #[error("logging: {0}")]
    Logging(String),
}
