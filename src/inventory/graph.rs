//! Deterministic directed acyclic graph (`DAG`) construction and role membership resolution.

use super::model::{DependencyGraph, GroupConfig};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(crate) fn build_dependency_graph(
    all_groups: &BTreeMap<String, GroupConfig>,
) -> Result<DependencyGraph, String> {
    let mut adj: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut in_degree: BTreeMap<String, usize> = BTreeMap::new();
    for name in all_groups.keys() {
        adj.insert(name.clone(), BTreeSet::new());
        in_degree.insert(name.clone(), 0);
    }

    for (parent, cfg) in all_groups {
        let mut children = BTreeSet::new();
        if let Some(ref child_map) = cfg.children {
            for k in child_map.keys() {
                children.insert(k.clone());
            }
        }
        if let Some(ref child_list) = cfg.groups {
            for k in child_list {
                children.insert(k.clone());
            }
        }
        for child in children {
            if !all_groups.contains_key(&child) {
                return Err(format!(
                    "Inventory configuration error: Parent group '{parent}' references undefined child group '{child}'"
                ));
            }
            if adj.entry(child).or_default().insert(parent.clone()) {
                let deg = in_degree.entry(parent.clone()).or_default();
                *deg = deg.saturating_add(1);
            }
        }
    }

    if all_groups.contains_key("all") {
        for name in all_groups.keys() {
            if name != "all"
                && adj
                    .entry(name.clone())
                    .or_default()
                    .insert("all".to_string())
            {
                let deg = in_degree.entry("all".to_string()).or_default();
                *deg = deg.saturating_add(1);
            }
        }
    }

    let mut ready = BTreeSet::new();
    for (node, &degree) in &in_degree {
        if degree == 0 {
            ready.insert(node.clone());
        }
    }

    let mut topo_order = Vec::with_capacity(all_groups.len());
    while let Some(u) = ready.pop_last() {
        if let Some(neighbors) = adj.get(&u) {
            for v in neighbors {
                if let Some(deg) = in_degree.get_mut(v) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        ready.insert(v.clone());
                    }
                }
            }
        }
        topo_order.push(u);
    }

    if topo_order.len() != all_groups.len() {
        return Err("Cyclic dependency detected in inventory groups".to_string());
    }
    Ok((topo_order, adj))
}

pub(crate) fn resolve_role_hosts_rec(
    role: &str,
    direct_memberships: &BTreeMap<String, Vec<String>>,
    hosts_map: &HashMap<String, Value>,
    visited: &mut BTreeSet<String>,
) -> Result<BTreeSet<String>, String> {
    if visited.contains(role) {
        return Err(format!("Circular dependency detected in roles: {role}"));
    }
    visited.insert(role.to_string());
    let mut hosts = BTreeSet::new();
    if let Some(members) = direct_memberships.get(role) {
        for m in members {
            if hosts_map.contains_key(m) {
                hosts.insert(m.clone());
            } else if direct_memberships.contains_key(m) {
                let sub_hosts = resolve_role_hosts_rec(m, direct_memberships, hosts_map, visited)?;
                hosts.extend(sub_hosts);
            } else {
                return Err(format!(
                    "Role '{role}' references undefined host or role '{m}'"
                ));
            }
        }
    }
    visited.remove(role);
    Ok(hosts)
}
