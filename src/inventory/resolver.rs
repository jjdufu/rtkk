//! Resolution of hierarchical and simple inventories into deterministic [`FlatInventory`] maps.

use super::graph::{build_dependency_graph, resolve_role_hosts_rec};
use super::model::{FlatInventory, GroupConfig, Inventory};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

fn merge_hosts_field(
    existing_hosts: &mut Option<HashMap<String, Value>>,
    incoming_hosts: &HashMap<String, Value>,
) {
    let base_map = existing_hosts.get_or_insert_with(HashMap::new);
    let mut keys: Vec<&String> = incoming_hosts.keys().collect();
    keys.sort();
    for k in keys {
        let Some(inc_val) = incoming_hosts.get(k) else {
            continue;
        };
        if inc_val.is_null() {
            base_map.entry(k.clone()).or_insert(Value::Null);
        } else if let Some(base_val) = base_map.get_mut(k) {
            if base_val.is_null() {
                base_val.clone_from(inc_val);
            } else {
                crate::utils::deep_merge(base_val, inc_val);
            }
        } else {
            base_map.insert(k.clone(), inc_val.clone());
        }
    }
}

fn merge_group_config(existing: &mut GroupConfig, incoming: &GroupConfig) {
    if let Some(ref incoming_hosts) = incoming.hosts {
        merge_hosts_field(&mut existing.hosts, incoming_hosts);
    }
    if let Some(ref incoming_children) = incoming.children {
        let base_children = existing.children.get_or_insert_with(HashMap::new);
        let mut keys: Vec<&String> = incoming_children.keys().collect();
        keys.sort();
        for cname in keys {
            let Some(opt_child) = incoming_children.get(cname) else {
                continue;
            };
            match (base_children.get_mut(cname), opt_child) {
                (Some(Some(base_c)), Some(inc_c)) => merge_group_config(base_c, inc_c),
                (Some(slot @ None), Some(inc_c)) => *slot = Some(inc_c.clone()),
                (Some(_), None) => {}
                (None, _) => {
                    base_children.insert(cname.clone(), opt_child.clone());
                }
            }
        }
    }
    if let Some(ref incoming_groups) = incoming.groups {
        let base_groups = existing.groups.get_or_insert_with(Vec::new);
        for g in incoming_groups {
            if !base_groups.contains(g) {
                base_groups.push(g.clone());
            }
        }
    }
    if let Some(ref incoming_vars) = incoming.vars {
        let base_vars = existing.vars.get_or_insert_with(HashMap::new);
        let mut keys: Vec<&String> = incoming_vars.keys().collect();
        keys.sort();
        for k in keys {
            let Some(inc_v) = incoming_vars.get(k) else {
                continue;
            };
            if let Some(base_v) = base_vars.get_mut(k) {
                crate::utils::deep_merge(base_v, inc_v);
            } else {
                base_vars.insert(k.clone(), inc_v.clone());
            }
        }
    }
}

pub(crate) fn collect_all_groups(
    current_name: &str,
    current_cfg: &GroupConfig,
    collected: &mut BTreeMap<String, GroupConfig>,
    visited_path: &mut Vec<String>,
) -> Result<(), String> {
    if visited_path.iter().any(|p| p == current_name) {
        visited_path.push(current_name.to_string());
        let path_str = visited_path.join(" -> ");
        return Err(format!(
            "Circular dependency detected in group hierarchy: {path_str}"
        ));
    }
    visited_path.push(current_name.to_string());
    if let Some(existing) = collected.get_mut(current_name) {
        merge_group_config(existing, current_cfg);
    } else {
        collected.insert(current_name.to_string(), current_cfg.clone());
    }
    if let Some(ref children) = current_cfg.children {
        let mut child_names: Vec<&String> = children.keys().collect();
        child_names.sort();
        let default_cfg = GroupConfig::default();
        for name in child_names {
            let child_cfg = children
                .get(name)
                .and_then(Option::as_ref)
                .unwrap_or(&default_cfg);
            collect_all_groups(name, child_cfg, collected, visited_path)?;
        }
    }
    visited_path.pop();
    Ok(())
}

fn resolve_group_host_sets(
    all_groups: &BTreeMap<String, GroupConfig>,
    topo_order: &[String],
    adj: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut group_hosts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, cfg) in all_groups {
        let mut hosts_set = BTreeSet::new();
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
                    p_hosts.extend(child_hosts.iter().cloned());
                }
            }
        }
    }
    group_hosts
}

fn finalize_groups_map(
    group_hosts: &BTreeMap<String, BTreeSet<String>>,
) -> HashMap<String, Vec<String>> {
    let mut grouped_hosts = BTreeSet::new();
    for (gname, hosts) in group_hosts {
        if gname != "all" && gname != "ungrouped" {
            grouped_hosts.extend(hosts.iter().cloned());
        }
    }
    let mut ungrouped_set = group_hosts.get("ungrouped").cloned().unwrap_or_default();
    if let Some(all_set) = group_hosts.get("all") {
        for h in all_set {
            if !grouped_hosts.contains(h) {
                ungrouped_set.insert(h.clone());
            }
        }
    }

    let mut resolved_groups = HashMap::with_capacity(group_hosts.len().saturating_add(2));
    for (gname, hset) in group_hosts {
        resolved_groups.insert(gname.clone(), hset.iter().cloned().collect());
    }
    resolved_groups.insert("ungrouped".to_string(), ungrouped_set.into_iter().collect());
    resolved_groups.entry("all".to_string()).or_default();
    resolved_groups
        .entry("localhost".to_string())
        .or_insert_with(|| vec!["localhost".to_string()]);
    resolved_groups
}

fn apply_host_value_vars(entry: &mut BTreeMap<String, Value>, hval: &Value) {
    if let Some(ip_str) = hval.as_str() {
        entry.insert("ip".to_string(), Value::String(ip_str.to_string()));
    } else if let Some(obj) = hval.as_object() {
        for (k, v) in obj {
            if let Some(existing) = entry.get_mut(k) {
                crate::utils::deep_merge(existing, v);
            } else {
                entry.insert(k.clone(), v.clone());
            }
        }
    }
}

fn collect_direct_host_vars(
    all_groups: &BTreeMap<String, GroupConfig>,
    parent_to_child_order: &[String],
) -> (BTreeSet<String>, BTreeMap<String, BTreeMap<String, Value>>) {
    let mut direct_host_vars: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    let mut explicit_hosts: BTreeSet<String> = BTreeSet::new();
    for gname in parent_to_child_order {
        let Some(cfg) = all_groups.get(gname) else {
            continue;
        };
        let Some(ref direct_hosts) = cfg.hosts else {
            continue;
        };
        let mut hnames: Vec<&String> = direct_hosts.keys().collect();
        hnames.sort();
        for hname in hnames {
            let Some(hval) = direct_hosts.get(hname) else {
                continue;
            };
            explicit_hosts.insert(hname.clone());
            let entry = direct_host_vars.entry(hname.clone()).or_default();
            apply_host_value_vars(entry, hval);
        }
    }
    (explicit_hosts, direct_host_vars)
}

fn merge_hashmap_vars_sorted(
    merged_obj: &mut serde_json::Map<String, Value>,
    vars: &HashMap<String, Value>,
) {
    let mut keys: Vec<&String> = vars.keys().collect();
    keys.sort();
    for k in keys {
        let Some(v) = vars.get(k) else {
            continue;
        };
        if let Some(existing) = merged_obj.get_mut(k) {
            crate::utils::deep_merge(existing, v);
        } else {
            merged_obj.insert(k.clone(), v.clone());
        }
    }
}

fn build_simple_host_vars(
    host_names: &[&String],
    hosts_map: &HashMap<String, Value>,
    global_vars: Option<&HashMap<String, Value>>,
) -> HashMap<String, HashMap<String, Value>> {
    let mut host_vars: HashMap<String, HashMap<String, Value>> =
        HashMap::with_capacity(host_names.len().saturating_add(1));
    for hname in host_names {
        let Some(hval) = hosts_map.get(*hname) else {
            continue;
        };
        let mut merged_obj = serde_json::Map::new();
        if let Some(g_vars) = global_vars {
            merge_hashmap_vars_sorted(&mut merged_obj, g_vars);
        }
        let mut direct = BTreeMap::new();
        apply_host_value_vars(&mut direct, hval);
        for (k, v) in direct {
            if let Some(existing) = merged_obj.get_mut(&k) {
                crate::utils::deep_merge(existing, &v);
            } else {
                merged_obj.insert(k, v);
            }
        }
        if *hname == "localhost" {
            merged_obj
                .entry("ansible_connection".to_string())
                .or_insert_with(|| Value::String("local".to_string()));
        }
        merged_obj.insert(
            "inventory_hostname".to_string(),
            Value::String((*hname).clone()),
        );
        host_vars.insert((*hname).clone(), merged_obj.into_iter().collect());
    }
    host_vars
}

fn resolve_simple_inventory(
    hosts_map: &HashMap<String, Value>,
    roles_map: &HashMap<String, Vec<String>>,
    global_vars: Option<&HashMap<String, Value>>,
) -> Result<FlatInventory, String> {
    let mut resolved_groups: HashMap<String, Vec<String>> = HashMap::new();
    let mut host_names: Vec<&String> = hosts_map.keys().collect();
    host_names.sort();
    let mut host_vars = build_simple_host_vars(&host_names, hosts_map, global_vars);

    let mut direct_memberships = BTreeMap::new();
    for (rname, members) in roles_map {
        direct_memberships.insert(rname.clone(), members.clone());
    }
    for rname in direct_memberships.keys() {
        let mut visited = BTreeSet::new();
        let hosts = resolve_role_hosts_rec(rname, &direct_memberships, hosts_map, &mut visited)?;
        resolved_groups.insert(rname.clone(), hosts.into_iter().collect());
    }

    let all_hosts: Vec<String> = host_names.iter().map(|h| (*h).clone()).collect();
    let mut grouped_hosts = BTreeSet::new();
    for (rname, hlist) in &resolved_groups {
        if rname != "all" && rname != "ungrouped" {
            grouped_hosts.extend(hlist.iter().cloned());
        }
    }
    let ungrouped_hosts: Vec<String> = all_hosts
        .iter()
        .filter(|h| !grouped_hosts.contains(*h))
        .cloned()
        .collect();

    resolved_groups.insert("all".to_string(), all_hosts);
    resolved_groups.insert("ungrouped".to_string(), ungrouped_hosts);
    resolved_groups
        .entry("localhost".to_string())
        .or_insert_with(|| vec!["localhost".to_string()]);

    if !host_vars.contains_key("localhost") {
        let mut local_obj = serde_json::Map::new();
        if let Some(g_vars) = global_vars {
            merge_hashmap_vars_sorted(&mut local_obj, g_vars);
        }
        local_obj.insert(
            "ansible_connection".to_string(),
            Value::String("local".to_string()),
        );
        local_obj.insert(
            "inventory_hostname".to_string(),
            Value::String("localhost".to_string()),
        );
        host_vars.insert("localhost".to_string(), local_obj.into_iter().collect());
    }

    Ok(FlatInventory {
        groups: resolved_groups,
        host_vars,
    })
}

fn merge_host_vars(
    explicit_hosts: &BTreeSet<String>,
    all_groups: &BTreeMap<String, GroupConfig>,
    group_hosts: &BTreeMap<String, BTreeSet<String>>,
    direct_host_vars: &BTreeMap<String, BTreeMap<String, Value>>,
    parent_to_child_order: &[String],
    global_vars: Option<&HashMap<String, Value>>,
) -> HashMap<String, HashMap<String, Value>> {
    let mut target_hosts = explicit_hosts.clone();
    target_hosts.insert("localhost".to_string());

    let mut host_vars = HashMap::with_capacity(target_hosts.len());
    for hname in &target_hosts {
        let mut merged_obj = serde_json::Map::new();
        if let Some(g_vars) = global_vars {
            merge_hashmap_vars_sorted(&mut merged_obj, g_vars);
        }
        if let Some(vars) = all_groups.get("all").and_then(|c| c.vars.as_ref()) {
            merge_hashmap_vars_sorted(&mut merged_obj, vars);
        }
        for gname in parent_to_child_order {
            if gname == "all" {
                continue;
            }
            if !group_hosts
                .get(gname)
                .is_some_and(|members| members.contains(hname))
            {
                continue;
            }
            if let Some(vars) = all_groups.get(gname).and_then(|c| c.vars.as_ref()) {
                merge_hashmap_vars_sorted(&mut merged_obj, vars);
            }
        }
        if let Some(direct_vars) = direct_host_vars.get(hname) {
            for (k, v) in direct_vars {
                if let Some(existing) = merged_obj.get_mut(k) {
                    crate::utils::deep_merge(existing, v);
                } else {
                    merged_obj.insert(k.clone(), v.clone());
                }
            }
        }
        if hname == "localhost" {
            merged_obj
                .entry("ansible_connection".to_string())
                .or_insert_with(|| Value::String("local".to_string()));
        }
        merged_obj.insert(
            "inventory_hostname".to_string(),
            Value::String(hname.clone()),
        );
        host_vars.insert(hname.clone(), merged_obj.into_iter().collect());
    }
    host_vars
}

/// Resolves a parsed [`Inventory`] into a [`FlatInventory`] with deterministic group member
/// lists and hierarchically merged host variables.
///
/// # Errors
///
/// Returns an error if undefined child groups or roles are referenced, or if a cyclic
/// dependency is detected in the group or role hierarchy.
pub fn resolve_inventory(inventory: &Inventory) -> Result<FlatInventory, String> {
    if let Some(ref hosts_map) = inventory.hosts {
        let empty_roles = HashMap::new();
        let roles_map = inventory.roles.as_ref().unwrap_or(&empty_roles);
        return resolve_simple_inventory(hosts_map, roles_map, inventory.vars.as_ref());
    }

    let mut all_groups = BTreeMap::new();
    let mut top_names: Vec<&String> = inventory.groups.keys().collect();
    top_names.sort();
    for name in top_names {
        if let Some(cfg) = inventory.groups.get(name) {
            let mut visited_path = Vec::new();
            collect_all_groups(name, cfg, &mut all_groups, &mut visited_path)?;
        }
    }
    if let Some(ref all_cfg) = inventory.all {
        let mut visited_path = Vec::new();
        collect_all_groups("all", all_cfg, &mut all_groups, &mut visited_path)?;
    } else if !all_groups.contains_key("all") {
        all_groups.insert("all".to_string(), GroupConfig::default());
    }

    let (topo_order, adj) = build_dependency_graph(&all_groups)?;
    let group_hosts = resolve_group_host_sets(&all_groups, &topo_order, &adj);
    let resolved_groups = finalize_groups_map(&group_hosts);

    let mut parent_to_child_order = topo_order;
    parent_to_child_order.reverse();
    let (explicit_hosts, direct_host_vars) =
        collect_direct_host_vars(&all_groups, &parent_to_child_order);
    let host_vars = merge_host_vars(
        &explicit_hosts,
        &all_groups,
        &group_hosts,
        &direct_host_vars,
        &parent_to_child_order,
        inventory.vars.as_ref(),
    );

    Ok(FlatInventory {
        groups: resolved_groups,
        host_vars,
    })
}
