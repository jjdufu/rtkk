//! Data structures representing raw `YAML` inventories and flattened host/group mappings.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Configuration of an inventory group, including direct hosts, child groups, and group variables.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct GroupConfig {
    /// Direct host definitions mapped from hostname to host variable object, IP string, or `null`.
    pub hosts: Option<HashMap<String, Value>>,
    /// Nested child groups (`None` when declared with an empty `YAML` body such as `kube_node:`).
    pub children: Option<HashMap<String, Option<GroupConfig>>>,
    /// Additional child group names referenced as a list.
    pub groups: Option<Vec<String>>,
    /// Variables shared by all hosts belonging to this group.
    pub vars: Option<HashMap<String, Value>>,
}

/// Top-level inventory document supporting both hierarchical `Ansible` groups and flat `hosts`/`roles`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct Inventory {
    /// Root `all` group configuration in hierarchical `YAML` inventories.
    pub all: Option<GroupConfig>,
    /// Top-level `hosts` map used in simple `hosts`/`roles` inventories.
    pub hosts: Option<HashMap<String, Value>>,
    /// Top-level `roles` membership map used in simple `hosts`/`roles` inventories.
    pub roles: Option<HashMap<String, Vec<String>>>,
    /// Global lowest-precedence variables applied across hosts.
    pub vars: Option<HashMap<String, Value>>,
    /// Additional top-level group definitions captured via `#[serde(flatten)]`.
    #[serde(flatten)]
    pub groups: HashMap<String, GroupConfig>,
}

/// Flattened, resolved inventory containing deterministic group member lists and merged host variables.
#[derive(Debug, Clone)]
pub struct FlatInventory {
    /// Mapping from group name to its sorted list of member hostnames.
    pub groups: HashMap<String, Vec<String>>,
    /// Mapping from hostname to its fully merged variable map.
    pub host_vars: HashMap<String, HashMap<String, Value>>,
}

pub(crate) type DependencyGraph = (Vec<String>, BTreeMap<String, BTreeSet<String>>);
