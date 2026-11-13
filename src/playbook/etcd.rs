//! Deterministic inventory group/etcd variable synthesis and runtime variable injection.

use serde_json::{Map, Value};
use std::collections::HashMap;
use std::hash::BuildHasher;

const SAMPLE_PLACEHOLDER_AUTH_BASIC: &str = "YWRtaW46Y2hhbmdlLW1l";

/// Injects deterministic inventory-derived variables (`groups`, `first_control_plane`,
/// `etcd_initial_cluster`, `etcd_endpoints_str`, and `etcd_endpoints`) into every host's variable map.
pub fn inject_dynamic_inventory_vars<S1, S2, S3>(
    resolved_groups: &HashMap<String, Vec<String>, S1>,
    host_vars: &mut HashMap<String, HashMap<String, Value, S3>, S2>,
) where
    S1: BuildHasher,
    S2: BuildHasher,
    S3: BuildHasher,
{
    inject_groups(resolved_groups, host_vars);
    inject_etcd_vars(resolved_groups, host_vars);
}

fn inject_groups<S1, S2, S3>(
    resolved_groups: &HashMap<String, Vec<String>, S1>,
    host_vars: &mut HashMap<String, HashMap<String, Value, S3>, S2>,
) where
    S1: BuildHasher,
    S2: BuildHasher,
    S3: BuildHasher,
{
    let mut group_names: Vec<&String> = resolved_groups.keys().collect();
    group_names.sort();
    let mut groups_map = Map::new();
    for gname in group_names {
        if let Some(hosts) = resolved_groups.get(gname) {
            groups_map.insert(
                gname.clone(),
                Value::Array(hosts.iter().map(|h| Value::String(h.clone())).collect()),
            );
        }
    }
    if !groups_map.contains_key("all") {
        let mut fallback_hosts: Vec<String> = host_vars.keys().cloned().collect();
        fallback_hosts.sort();
        groups_map.insert(
            "all".to_string(),
            Value::Array(fallback_hosts.into_iter().map(Value::String).collect()),
        );
    }
    let groups_value = Value::Object(groups_map);

    let first_control_plane = resolved_groups
        .get("kube_control_plane")
        .and_then(|v| v.first())
        .cloned()
        .unwrap_or_default();

    for h_vars in host_vars.values_mut() {
        h_vars.insert("groups".to_string(), groups_value.clone());
        if !first_control_plane.is_empty() {
            h_vars.insert(
                "first_control_plane".to_string(),
                Value::String(first_control_plane.clone()),
            );
        }
    }
}

fn inject_etcd_vars<S1, S2, S3>(
    resolved_groups: &HashMap<String, Vec<String>, S1>,
    host_vars: &mut HashMap<String, HashMap<String, Value, S3>, S2>,
) where
    S1: BuildHasher,
    S2: BuildHasher,
    S3: BuildHasher,
{
    let mut etcd_hosts: Vec<String> = resolved_groups
        .get("etcd")
        .filter(|v| !v.is_empty())
        .or_else(|| {
            resolved_groups
                .get("kube_control_plane")
                .filter(|v| !v.is_empty())
        })
        .cloned()
        .unwrap_or_default();
    if etcd_hosts.is_empty() {
        return;
    }
    etcd_hosts.sort();
    etcd_hosts.dedup();
    let mut initial_cluster_parts = Vec::with_capacity(etcd_hosts.len());
    let mut endpoint_parts = Vec::with_capacity(etcd_hosts.len());
    let mut endpoints_array = Vec::with_capacity(etcd_hosts.len());
    for hname in &etcd_hosts {
        let ip = host_vars
            .get(hname)
            .and_then(|vars| vars.get("ip"))
            .and_then(Value::as_str)
            .unwrap_or(hname);
        initial_cluster_parts.push(format!("{hname}=https://{ip}:2380"));
        endpoint_parts.push(format!("https://{ip}:2379"));
        endpoints_array.push(Value::String(format!("https://{ip}:2379")));
    }
    let etcd_initial_cluster = initial_cluster_parts.join(",");
    let etcd_endpoints_str = endpoint_parts.join(",");
    let etcd_endpoints = Value::Array(endpoints_array);
    for h_vars in host_vars.values_mut() {
        h_vars.insert(
            "etcd_initial_cluster".to_string(),
            Value::String(etcd_initial_cluster.clone()),
        );
        h_vars.insert(
            "etcd_endpoints_str".to_string(),
            Value::String(etcd_endpoints_str.clone()),
        );
        h_vars.insert("etcd_endpoints".to_string(), etcd_endpoints.clone());
    }
}

fn is_unset_string_var(val: Option<&Value>) -> bool {
    match val {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.trim().is_empty(),
        Some(_) => false,
    }
}

fn sync_registry_auth_basic(reg_obj: &mut Map<String, Value>) {
    let username = reg_obj
        .get("username")
        .and_then(Value::as_str)
        .unwrap_or("");
    let password = reg_obj
        .get("password")
        .and_then(Value::as_str)
        .unwrap_or("");
    if username.is_empty() {
        return;
    }
    let computed = crate::utils::base64_encode(format!("{username}:{password}").as_bytes());
    let existing = reg_obj
        .get("auth_basic")
        .and_then(Value::as_str)
        .map(str::trim);
    let should_update = match existing {
        None | Some("") => true,
        Some(curr) => curr == SAMPLE_PLACEHOLDER_AUTH_BASIC && computed != curr,
    };
    if should_update {
        reg_obj.insert("auth_basic".to_string(), Value::String(computed));
    }
}

/// Injects runtime variables (`inventory_file`, `rtkk_exe`, and `registry.auth_basic`)
/// into every host's variable map prior to playbook execution.
pub fn inject_runtime_vars<S1, S2>(
    host_vars: &mut HashMap<String, HashMap<String, Value, S2>, S1>,
    inventory_path: &str,
) where
    S1: BuildHasher,
    S2: BuildHasher,
{
    let exe_str = std::env::current_exe()
        .ok()
        .map_or_else(|| "rtkk".to_string(), |p| p.to_string_lossy().into_owned());
    for h_vars in host_vars.values_mut() {
        if is_unset_string_var(h_vars.get("inventory_file")) {
            h_vars.insert(
                "inventory_file".to_string(),
                Value::String(inventory_path.to_string()),
            );
        }
        if is_unset_string_var(h_vars.get("rtkk_exe")) {
            h_vars.insert("rtkk_exe".to_string(), Value::String(exe_str.clone()));
        }
        if let Some(Value::Object(reg_obj)) = h_vars.get_mut("registry") {
            sync_registry_auth_basic(reg_obj);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::inject_runtime_vars;
    use serde_json::{Value, json};
    use std::collections::HashMap;

    #[test]
    fn runtime_vars_populates_and_rotates_placeholder_auth_basic() -> Result<(), String> {
        let mut host_vars: HashMap<String, HashMap<String, Value>> = HashMap::new();
        let mut h1 = HashMap::new();
        h1.insert(
            "registry".to_string(),
            json!({
                "username": "admin",
                "password": "new-password",
                "auth_basic": "YWRtaW46Y2hhbmdlLW1l"
            }),
        );
        host_vars.insert("node1".to_string(), h1);
        inject_runtime_vars(&mut host_vars, "/etc/rtkk/inventory.yaml");
        let n1 = host_vars.get("node1").ok_or("missing node1")?;
        if n1.get("inventory_file").and_then(Value::as_str) != Some("/etc/rtkk/inventory.yaml") {
            return Err("inventory_file not injected".to_string());
        }
        let auth = n1
            .get("registry")
            .and_then(|r| r.get("auth_basic"))
            .and_then(Value::as_str);
        if auth != Some("YWRtaW46bmV3LXBhc3N3b3Jk") {
            return Err(format!("expected rotated auth_basic, got {auth:?}"));
        }
        Ok(())
    }
}
