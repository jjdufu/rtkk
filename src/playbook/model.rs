//! Playbook and Play `YAML` data structures.

use crate::connector::BecomeMethod;
use crate::tasks::Task;
use serde::Deserializer;
use serde_json::Value;
use std::collections::HashMap;

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum HostsSpec {
    Single(String),
    List(Vec<String>),
}

fn deserialize_hosts<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let spec: HostsSpec = serde::Deserialize::deserialize(deserializer)?;
    match spec {
        HostsSpec::Single(s) => Ok(s),
        HostsSpec::List(items) => {
            let parts: Vec<String> = items
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Ok(parts.join(","))
        }
    }
}

/// Ordered sequence of plays compiled from a `YAML` playbook file.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Playbook(
    /// Ordered list of plays to execute.
    pub Vec<Play>,
);

/// A single play targeting one or more inventory groups or hosts.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Play {
    /// Optional human-readable play title.
    #[serde(default)]
    pub name: Option<String>,
    /// Target host or group pattern (deserializable from a `YAML` string or list of strings).
    #[serde(deserialize_with = "deserialize_hosts")]
    pub hosts: String,
    /// Optional rolling batch specification (`usize`, percentage string such as `"50%"`, or list).
    #[serde(default)]
    pub serial: Option<Value>,
    /// Play-scoped variables merged into target host variables.
    #[serde(default)]
    pub vars: Option<HashMap<String, Value>>,
    /// Roles executed before `tasks`.
    #[serde(default)]
    pub roles: Option<Vec<Value>>,
    /// Tasks executed after any `roles` in this play.
    #[serde(default)]
    pub tasks: Option<Vec<Task>>,
    /// Whether task failures in this play should be ignored instead of aborting execution.
    #[serde(default)]
    pub ignore_errors: Option<bool>,
    /// Play-level privilege escalation (`sudo` / `su`) toggle inherited by tasks in this play.
    #[serde(default, rename = "become")]
    pub r#become: Option<bool>,
    /// Play-level privilege escalation mechanism (`sudo` or `su`).
    #[serde(default)]
    pub become_method: Option<BecomeMethod>,
    /// Target user for play-level privilege escalation (`"root"` when unset).
    #[serde(default)]
    pub become_user: Option<String>,
}
