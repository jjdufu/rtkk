use serde_json::Value;
use std::collections::{HashMap, HashSet};
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub(crate) struct GroupConfig {
    pub hosts: Option<HashMap<String, Value>>,
    pub children: Option<HashMap<String, Value>>,
    pub groups: Option<Vec<String>>,
    pub vars: Option<HashMap<String, Value>>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub(crate) struct Inventory {
    pub all: Option<GroupConfig>,
    pub hosts: Option<HashMap<String, Value>>,
    pub roles: Option<HashMap<String, Vec<String>>>,
    pub vars: Option<HashMap<String, Value>>,
    #[serde(flatten)]
    pub groups: HashMap<String, GroupConfig>,
}
#[derive(Debug, Clone)]
pub(crate) struct FlatInventory {
    pub groups: HashMap<String, Vec<String>>,
    pub host_vars: HashMap<String, HashMap<String, Value>>,
}
pub(crate) type DependencyGraph = (Vec<String>, HashMap<String, HashSet<String>>);
