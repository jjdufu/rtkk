use serde_json::{Map, Value};
use std::collections::HashMap;

/// Inject inventory-derived variables used by playbooks (etcd cluster endpoints,
/// role group membership, first control-plane hostname).
pub(crate) fn inject_dynamic_inventory_vars(
    resolved_groups: &HashMap<String, Vec<String>>,
    host_vars: &mut HashMap<String, HashMap<String, Value>>,
) {
    inject_groups(resolved_groups, host_vars);
    inject_etcd_vars(resolved_groups, host_vars);
}

fn inject_groups(
    resolved_groups: &HashMap<String, Vec<String>>,
    host_vars: &mut HashMap<String, HashMap<String, Value>>,
) {
    let mut groups_map = Map::new();
    for (gname, hosts) in resolved_groups {
        groups_map.insert(
            gname.clone(),
            Value::Array(hosts.iter().map(|h| Value::String(h.clone())).collect()),
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

fn inject_etcd_vars(
    resolved_groups: &HashMap<String, Vec<String>>,
    host_vars: &mut HashMap<String, HashMap<String, Value>>,
) {
    let etcd_hosts = resolved_groups
        .get("etcd")
        .or_else(|| resolved_groups.get("kube_control_plane"))
        .cloned()
        .unwrap_or_default();
    if etcd_hosts.is_empty() {
        return;
    }
    let mut initial_cluster_parts = Vec::new();
    let mut endpoint_parts = Vec::new();
    let mut endpoints_array = Vec::new();
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
