use super::model::{DependencyGraph, GroupConfig};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
pub(crate) fn build_dependency_graph(
    all_groups: &HashMap<String, GroupConfig>,
) -> Result<DependencyGraph, String> {
    let mut adj: HashMap<String, HashSet<String>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    for name in all_groups.keys() {
        adj.insert(name.clone(), HashSet::new());
        in_degree.insert(name.clone(), 0);
    }
    for (parent, cfg) in all_groups {
        let mut children = HashSet::new();
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
            if adj.entry(child.clone()).or_default().insert(parent.clone()) {
                *in_degree.entry(parent.clone()).or_default() += 1;
            }
        }
    }
    let mut queue = VecDeque::new();
    for (node, &degree) in &in_degree {
        if degree == 0 {
            queue.push_back(node.clone());
        }
    }
    let mut topo_order = Vec::new();
    while let Some(u) = queue.pop_front() {
        topo_order.push(u.clone());
        if let Some(neighbors) = adj.get(&u) {
            for v in neighbors {
                if let Some(deg) = in_degree.get_mut(v) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(v.clone());
                    }
                }
            }
        }
    }
    if topo_order.len() != all_groups.len() {
        return Err("Cyclic dependency detected in inventory groups".to_string());
    }
    Ok((topo_order, adj))
}
pub(crate) fn resolve_role_hosts_rec(
    role: &str,
    direct_memberships: &HashMap<String, Vec<String>>,
    hosts_map: &HashMap<String, Value>,
    visited: &mut HashSet<String>,
) -> Result<HashSet<String>, String> {
    if visited.contains(role) {
        return Err(format!("Circular dependency detected in roles: {role}"));
    }
    visited.insert(role.to_string());
    let mut hosts = HashSet::new();
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
