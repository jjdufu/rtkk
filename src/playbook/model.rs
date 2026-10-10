use crate::tasks::Task;
use serde_json::Value;
use std::collections::HashMap;
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Playbook(pub Vec<Play>);
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Play {
    pub name: Option<String>,
    pub hosts: String,
    pub serial: Option<Value>,
    pub vars: Option<HashMap<String, Value>>,
    pub roles: Option<Vec<Value>>,
    pub tasks: Option<Vec<Task>>,
    pub ignore_errors: Option<bool>,
}
