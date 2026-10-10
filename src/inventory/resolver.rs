use super::graph::{build_dependency_graph, resolve_role_hosts_rec};
use super::model::{FlatInventory, GroupConfig, Inventory};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
pub(crate) fn collect_all_groups(
    current_name: &str,
    current_cfg: &GroupConfig,
    collected: &mut HashMap<String, GroupConfig>,
    visited_path: &mut Vec<String>,
) -> Result<(), String> {
    if visited_path.contains(&current_name.to_string()) {
        visited_path.push(current_name.to_string());
        let path_str = visited_path.join(" -> ");
        return Err(format!(
            "Circular dependency detected in group hierarchy: {path_str}"
        ));
    }
    visited_path.push(current_name.to_string());
    if let Some(existing) = collected.get_mut(current_name) {
        if let Some(ref incoming_hosts) = current_cfg.hosts {
            if let Some(ref mut base_hosts) = existing.hosts {
                base_hosts.extend(incoming_hosts.clone());
            } else {
                existing.hosts = Some(incoming_hosts.clone());
            }
        }
        if let Some(ref incoming_children) = current_cfg.children {
            if let Some(ref mut base_children) = existing.children {
                base_children.extend(incoming_children.clone());
            } else {
                existing.children = Some(incoming_children.clone());
            }
        }
        if let Some(ref incoming_groups) = current_cfg.groups {
            if let Some(ref mut base_groups) = existing.groups {
                for g in incoming_groups {
                    if !base_groups.contains(g) {
                        base_groups.push(g.clone());
                    }
                }
            } else {
                existing.groups = Some(incoming_groups.clone());
            }
        }
        if let Some(ref incoming_vars) = current_cfg.vars {
            if let Some(ref mut base_vars) = existing.vars {
                base_vars.extend(incoming_vars.clone());
            } else {
                existing.vars = Some(incoming_vars.clone());
            }
        }
    } else {
        collected.insert(current_name.to_string(), current_cfg.clone());
    }
    if let Some(ref children) = current_cfg.children {
        for (name, val) in children {
            let child_cfg = serde_json::from_value::<GroupConfig>(val.clone())
                .map_err(|e| format!("Failed to parse group config for '{name}': {e}"))?;
            collect_all_groups(name, &child_cfg, collected, visited_path)?;
        }
    }
    visited_path.pop();
    Ok(())
}
pub(crate) fn resolve_group_hosts(
    all_groups: &HashMap<String, GroupConfig>,
    topo_order: &[String],
    adj: &HashMap<String, HashSet<String>>,
) -> HashMap<String, Vec<String>> {
    let mut group_hosts: HashMap<String, HashSet<String>> = HashMap::new();
    for (name, cfg) in all_groups {
        let mut hosts_set = HashSet::new();
        if let Some(ref direct_hosts) = cfg.hosts {
            for hname in direct_hosts.keys() {
                hosts_set.insert(hname.clone());
            }
        }
        group_hosts.insert(name.clone(), hosts_set);
    }
    for child in topo_order {
        let child_hosts = group_hosts.get(child).cloned().unwrap_or_default();
        if let Some(parents) = adj.get(child) {
            for parent in parents {
                if let Some(p_hosts) = group_hosts.get_mut(parent) {
                    p_hosts.extend(child_hosts.clone());
                }
            }
        }
    }
    let mut resolved_groups: HashMap<String, Vec<String>> = HashMap::new();
    for (gname, hset) in group_hosts {
        let mut list: Vec<String> = hset.into_iter().collect();
        list.sort();
        resolved_groups.insert(gname, list);
    }
    resolved_groups
}
pub(crate) fn handle_ungrouped_and_localhost(resolved_groups: &mut HashMap<String, Vec<String>>) {
    let mut all_hosts: HashSet<String> = HashSet::new();
    for (gname, hosts) in resolved_groups.iter() {
        if gname != "all" && gname != "ungrouped" {
            all_hosts.extend(hosts.clone());
        }
    }
    let mut ungrouped_hosts = Vec::new();
    if let Some(all_list) = resolved_groups.get("all") {
        for h in all_list {
            if !all_hosts.contains(h) {
                ungrouped_hosts.push(h.clone());
            }
        }
    }
    ungrouped_hosts.sort();
    resolved_groups.insert("ungrouped".to_string(), ungrouped_hosts);
    let all_list = resolved_groups.entry("all".to_string()).or_default();
    if !all_list.contains(&"localhost".to_string()) {
        all_list.push("localhost".to_string());
    }
}
pub(crate) fn collect_direct_host_vars(
    all_groups: &HashMap<String, GroupConfig>,
) -> (HashSet<String>, HashMap<String, HashMap<String, Value>>) {
    let mut direct_host_vars: HashMap<String, HashMap<String, Value>> = HashMap::new();
    let mut all_hostnames: HashSet<String> = HashSet::new();
    for cfg in all_groups.values() {
        if let Some(ref direct_hosts) = cfg.hosts {
            for (hname, hval) in direct_hosts {
                all_hostnames.insert(hname.clone());
                let entry = direct_host_vars.entry(hname.clone()).or_default();
                if let Some(obj) = hval.as_object() {
                    for (k, v) in obj {
                        entry.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }
    all_hostnames.insert("localhost".to_string());
    (all_hostnames, direct_host_vars)
}
pub(crate) fn resolve_simple_inventory(
    hosts_map: &HashMap<String, Value>,
    roles_map: &HashMap<String, Vec<String>>,
    global_vars: Option<&HashMap<String, Value>>,
) -> Result<FlatInventory, String> {
    let mut resolved_groups: HashMap<String, Vec<String>> = HashMap::new();
    let mut host_vars: HashMap<String, HashMap<String, Value>> = HashMap::new();
    for (hname, hval) in hosts_map {
        let mut vars = HashMap::new();
        if let Some(g_vars) = global_vars {
            for (k, v) in g_vars {
                vars.insert(k.clone(), v.clone());
            }
        }
        if let Some(ip_str) = hval.as_str() {
            vars.insert("ip".to_string(), Value::String(ip_str.to_string()));
        } else if let Some(obj) = hval.as_object() {
            for (k, v) in obj {
                vars.insert(k.clone(), v.clone());
            }
        }
        vars.insert(
            "inventory_hostname".to_string(),
            Value::String(hname.clone()),
        );
        host_vars.insert(hname.clone(), vars);
    }
    let mut direct_memberships = HashMap::new();
    for (rname, members) in roles_map {
        direct_memberships.insert(rname.clone(), members.clone());
    }
    for rname in roles_map.keys() {
        let mut visited = HashSet::new();
        let hosts = resolve_role_hosts_rec(rname, &direct_memberships, hosts_map, &mut visited)?;
        let mut hosts_vec: Vec<String> = hosts.into_iter().collect();
        hosts_vec.sort();
        resolved_groups.insert(rname.clone(), hosts_vec);
    }
    let mut all_hosts: Vec<String> = hosts_map.keys().cloned().collect();
    all_hosts.sort();
    if !all_hosts.contains(&"localhost".to_string()) {
        all_hosts.push("localhost".to_string());
    }
    resolved_groups.insert("all".to_string(), all_hosts);
    let mut ungrouped_hosts = Vec::new();
    let mut grouped_hosts = HashSet::new();
    for (rname, hlist) in &resolved_groups {
        if rname != "all" && rname != "ungrouped" {
            grouped_hosts.extend(hlist.clone());
        }
    }
    for hname in hosts_map.keys() {
        if !grouped_hosts.contains(hname) {
            ungrouped_hosts.push(hname.clone());
        }
    }
    ungrouped_hosts.sort();
    resolved_groups.insert("ungrouped".to_string(), ungrouped_hosts);
    if !host_vars.contains_key("localhost") {
        let mut local_vars = HashMap::new();
        if let Some(g_vars) = global_vars {
            for (k, v) in g_vars {
                local_vars.insert(k.clone(), v.clone());
            }
        }
        local_vars.insert(
            "inventory_hostname".to_string(),
            Value::String("localhost".to_string()),
        );
        host_vars.insert("localhost".to_string(), local_vars);
    }
    Ok(FlatInventory {
        groups: resolved_groups,
        host_vars,
    })
}
pub(crate) fn merge_host_vars(
    all_hostnames: &HashSet<String>,
    all_groups: &HashMap<String, GroupConfig>,
    resolved_groups: &HashMap<String, Vec<String>>,
    direct_host_vars: &HashMap<String, HashMap<String, Value>>,
    parent_to_child_order: &[String],
) -> HashMap<String, HashMap<String, Value>> {
    let mut host_to_groups: HashMap<String, HashSet<String>> = HashMap::new();
    for (gname, hlist) in resolved_groups {
        for hname in hlist {
            host_to_groups
                .entry(hname.clone())
                .or_default()
                .insert(gname.clone());
        }
    }
    let mut host_vars = HashMap::new();
    for hname in all_hostnames {
        let mut merged_val = serde_json::Value::Object(serde_json::Map::new());
        if let Some(all_cfg) = all_groups.get("all") {
            if let Some(ref vars) = all_cfg.vars {
                let vars_val = serde_json::to_value(vars).unwrap_or(serde_json::Value::Null);
                crate::utils::deep_merge(&mut merged_val, &vars_val);
            }
        }
        let default_set = HashSet::new();
        let groups_for_host = host_to_groups.get(hname).unwrap_or(&default_set);
        for gname in parent_to_child_order {
            if gname == "all" {
                continue;
            }
            let is_in_group = groups_for_host.contains(gname);
            if is_in_group {
                if let Some(cfg) = all_groups.get(gname) {
                    if let Some(ref vars) = cfg.vars {
                        let vars_val =
                            serde_json::to_value(vars).unwrap_or(serde_json::Value::Null);
                        crate::utils::deep_merge(&mut merged_val, &vars_val);
                    }
                }
            }
        }
        if let Some(direct_vars) = direct_host_vars.get(hname) {
            let vars_val = serde_json::to_value(direct_vars).unwrap_or(serde_json::Value::Null);
            crate::utils::deep_merge(&mut merged_val, &vars_val);
        }
        if let Some(obj) = merged_val.as_object_mut() {
            obj.insert(
                "inventory_hostname".to_string(),
                Value::String(hname.clone()),
            );
        }
        if let serde_json::Value::Object(obj) = merged_val {
            let mut h_map = HashMap::new();
            for (k, v) in obj {
                h_map.insert(k, v);
            }
            host_vars.insert(hname.clone(), h_map);
        }
    }
    host_vars
}
pub(crate) fn resolve_inventory(inventory: &Inventory) -> Result<FlatInventory, String> {
    if let Some(ref hosts_map) = inventory.hosts {
        let empty_roles = HashMap::new();
        let roles_map = inventory.roles.as_ref().unwrap_or(&empty_roles);
        return resolve_simple_inventory(hosts_map, roles_map, inventory.vars.as_ref());
    }
    let mut all_groups = HashMap::new();
    for (name, cfg) in &inventory.groups {
        let mut visited_path = Vec::new();
        collect_all_groups(name, cfg, &mut all_groups, &mut visited_path)?;
    }
    if let Some(ref all_cfg) = inventory.all {
        let mut visited_path = Vec::new();
        collect_all_groups("all", all_cfg, &mut all_groups, &mut visited_path)?;
    } else if !all_groups.contains_key("all") {
        all_groups.insert("all".to_string(), GroupConfig::default());
    }
    let (topo_order, adj) = build_dependency_graph(&all_groups)?;
    let mut resolved_groups = resolve_group_hosts(&all_groups, &topo_order, &adj);
    handle_ungrouped_and_localhost(&mut resolved_groups);
    let (all_hostnames, direct_host_vars) = collect_direct_host_vars(&all_groups);
    let mut parent_to_child_order = topo_order.clone();
    parent_to_child_order.reverse();
    let host_vars = merge_host_vars(
        &all_hostnames,
        &all_groups,
        &resolved_groups,
        &direct_host_vars,
        &parent_to_child_order,
    );
    Ok(FlatInventory {
        groups: resolved_groups,
        host_vars,
    })
}
